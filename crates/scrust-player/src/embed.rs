//! Embedded host (`--embed`): no window. A parent process - the GUI's
//! Electron main process, see `gui/electron` - draws the frames and feeds the
//! input. Sound still plays here, through rodio.
//!
//! stdin, one command per line:
//!   `move X Y`              mouse position (Scratch coordinates)
//!   `button X Y 0|1`        mouse button up / down
//!   `key 0|1 HEX`           key up / down; HEX is the UTF-8 of a DOM `KeyboardEvent.key`
//!   `flag` / `stop`         green flag / stop sign
//!   `state`                 ask for a state message
//! End of input ends the program.
//!
//! stdout, binary messages `[tag: u8][length: u32 LE][payload]`:
//!   `F` frame: width u32 LE, height u32 LE, RGBA pixels (only when the picture changed)
//!   `T` number of running threads, u32 LE (when it changes)
//!   `S` state JSON (`crate::state`), on request and whenever the program goes idle

use crate::audio::RodioAudio;
use crate::input::key_event;
use scrust_render::SoftRenderer;
use scrust_runtime::cli::{install_native_clock, Args};
use scrust_runtime::{set_world_ptr, ProgramDef, World};
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

enum Cmd {
    Move(f64, f64),
    Button(f64, f64, bool),
    Key(String, bool),
    Flag,
    Stop,
    State,
}

fn parse(line: &str) -> Option<Cmd> {
    let mut it = line.split(' ');
    let num = |s: Option<&str>| s.and_then(|v| v.parse::<f64>().ok());
    Some(match it.next()? {
        "move" => Cmd::Move(num(it.next())?, num(it.next())?),
        "button" => Cmd::Button(num(it.next())?, num(it.next())?, it.next()? == "1"),
        "key" => {
            let down = it.next()? == "1";
            let hex = it.next()?;
            let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect();
            Cmd::Key(String::from_utf8(bytes?).ok()?, down)
        }
        "flag" => Cmd::Flag,
        "stop" => Cmd::Stop,
        "state" => Cmd::State,
        _ => return None,
    })
}

fn send(out: &mut impl Write, tag: u8, parts: &[&[u8]]) -> std::io::Result<()> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    out.write_all(&[tag])?;
    out.write_all(&(len as u32).to_le_bytes())?;
    for p in parts {
        out.write_all(p)?;
    }
    Ok(())
}

pub fn main(program: &'static ProgramDef, args: Args, scale: f64) {
    let mut world = World::new(program, args.options.clone());
    install_native_clock(&mut world);
    world.current_msecs = world.clock_ms();
    world.timer_start = world.current_msecs;
    world.audio = RodioAudio::new(program).map(|a| Box::new(a) as Box<dyn scrust_runtime::Audio>);
    world.renderer = Some(Box::new(SoftRenderer::new(program, scale.clamp(0.25, 4.0))));

    let (tx, rx) = mpsc::channel::<Cmd>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Some(cmd) = parse(line.trim_end()) {
                if tx.send(cmd).is_err() {
                    break;
                }
            }
        }
        // Dropping `tx` tells the main loop that the parent is gone.
    });

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, stdout.lock());
    set_world_ptr(&mut *world);
    world.green_flag();

    let step = Duration::from_secs_f64(1.0 / args.options.fps);
    let mut next_tick = Instant::now();
    let mut last_threads: Option<u32> = None;
    let mut last_frame: Vec<u8> = Vec::new();
    let mut rgba: Vec<u8> = Vec::new();
    loop {
        let wait = next_tick.saturating_duration_since(Instant::now());
        let mut cmds = Vec::new();
        match rx.recv_timeout(wait) {
            Ok(c) => cmds.push(c),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        cmds.extend(rx.try_iter());
        set_world_ptr(&mut *world);
        let mut want_state = false;
        for c in cmds {
            match c {
                Cmd::Move(x, y) => world.post_mouse_move(x, y),
                Cmd::Button(x, y, down) => world.post_mouse_button(x, y, down),
                Cmd::Key(key, down) => {
                    let text = if key.chars().count() == 1 { Some(key.clone()) } else { None };
                    key_event(&mut world, &key, text.as_deref(), down);
                }
                Cmd::Flag => world.green_flag(),
                Cmd::Stop => world.stop_all(),
                Cmd::State => want_state = true,
            }
        }

        let now = Instant::now();
        if now >= next_tick {
            let wp: *mut World = &mut *world;
            // SAFETY: the world is only accessed through `wp` during the step.
            unsafe { scrust_runtime::step(wp) };
            next_tick += step;
            if next_tick < now {
                next_tick = now + step;
            }
            let Some(mut r) = world.renderer.take() else { return };
            if let Some(sr) = r.as_any_mut().downcast_mut::<SoftRenderer>() {
                sr.draw(&world);
                rgba.clear();
                rgba.reserve(sr.frame.len() * 4);
                for p in &sr.frame {
                    rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]);
                }
                if rgba != last_frame {
                    let dims = [(sr.width as u32).to_le_bytes(), (sr.height as u32).to_le_bytes()].concat();
                    if send(&mut out, b'F', &[&dims, &rgba]).is_err() {
                        return;
                    }
                    std::mem::swap(&mut rgba, &mut last_frame);
                }
            }
            world.renderer = Some(r);
            let threads = world.live_threads() as u32;
            if last_threads != Some(threads) {
                if threads == 0 && last_threads.is_some_and(|n| n > 0) {
                    want_state = true;
                }
                if send(&mut out, b'T', &[&threads.to_le_bytes()]).is_err() {
                    return;
                }
                last_threads = Some(threads);
            }
        }
        if want_state && send(&mut out, b'S', &[crate::state::to_json(&world).as_bytes()]).is_err() {
            return;
        }
        if out.flush().is_err() {
            return;
        }
    }
}
