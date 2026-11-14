//! Headless command line driver for generated programs.

use crate::world::Options;
#[cfg(not(target_arch = "wasm32"))]
use crate::world::{ProgramDef, World};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;

pub struct Args {
    pub options: Options,
    pub trace: Option<u64>,
    /// With `--trace`: print only every Nth frame's line.
    pub trace_every: u64,
    pub frames: Option<u64>,
    pub bench: bool,
    pub print_say: bool,
    pub virtual_audio: bool,
}

/// Parse the common run-time flags. Unknown flags are returned untouched.
pub fn parse_args(mut options: Options, argv: &[String]) -> (Args, Vec<String>) {
    let mut a = Args { options: options.clone(), trace: None, trace_every: 1, frames: None, bench: false, print_say: false, virtual_audio: false };
    let mut rest = Vec::new();
    let mut i = 0;
    let next = |i: &mut usize| -> Option<String> {
        *i += 1;
        argv.get(*i).cloned()
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--trace" => a.trace = next(&mut i).and_then(|v| v.parse().ok()),
            "--trace-every" => a.trace_every = next(&mut i).and_then(|v| v.parse().ok()).unwrap_or(1).max(1),
            "--frames" => a.frames = next(&mut i).and_then(|v| v.parse().ok()),
            "--seed" => options.seed = next(&mut i).and_then(|v| v.parse().ok()).unwrap_or(1),
            "--fps" => options.fps = next(&mut i).and_then(|v| v.parse().ok()).unwrap_or(30.0),
            "--max-passes" => options.max_passes = next(&mut i).and_then(|v| v.parse().ok()),
            "--turbo" => options.turbo = true,
            "--deterministic" => options.deterministic = true,
            "--warp-timer" => options.warp_timer = true,
            "--answers" => options.answers = next(&mut i).map(|v| v.split('|').map(String::from).collect()).unwrap_or_default(),
            "--bench" => a.bench = true,
            "--print-say" => a.print_say = true,
            "--virtual-audio" => a.virtual_audio = true,
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    if options.deterministic && options.max_passes.is_none() {
        options.max_passes = Some(1);
    }
    a.options = options;
    (a, rest)
}

/// Install real-time clocks (native targets only).
#[cfg(not(target_arch = "wasm32"))]
pub fn install_native_clock(w: &mut World) {
    let start = std::time::Instant::now();
    w.host_clock = Some(Box::new(move || start.elapsed().as_secs_f64() * 1000.0));
    w.host_epoch_ms = Some(Box::new(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn main(program: ProgramDef, defaults: Options) {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (args, _rest) = parse_args(defaults, &argv);
    let program: &'static ProgramDef = Box::leak(Box::new(program));
    let mut w = World::new(program, args.options.clone());
    if !args.options.deterministic {
        install_native_clock(&mut w);
        w.current_msecs = w.clock_ms();
        w.timer_start = w.current_msecs;
    }
    if args.virtual_audio {
        w.audio = Some(Box::new(crate::vaudio::VirtualAudio::new(program)));
    }
    let frames = args.trace.or(args.frames).unwrap_or(u64::MAX);
    let trace = args.trace.is_some();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let started = std::time::Instant::now();
    let mut last_say: Vec<String> = Vec::new();
    let print_say = args.print_say;
    let realtime = !args.options.deterministic && args.trace.is_none() && args.frames.is_none();
    let step_ms = 1000.0 / args.options.fps;
    let mut next_tick = std::time::Instant::now();
    let mut done_frames = 0u64;
    crate::run_headless(&mut w, 0, |_| {});
    let wp: *mut World = &mut *w;
    // From here on the world is only reached through `wp`.
    // SAFETY: `w` (the owner) is not used again until the loop is over.
    let w: &World = unsafe { &*wp };
    while done_frames < frames {
        // SAFETY: `w` is only accessed through `wp` while stepping.
        unsafe { crate::sched::step(wp) };
        done_frames += 1;
        if trace && done_frames % args.trace_every == 0 {
            let _ = writeln!(out, "{}", w.trace_line());
        }
        if print_say {
            let cur: Vec<String> = w
                .order
                .iter()
                .map(|t| w.target(*t).bubble.as_ref().map(|b| b.text.to_rust_string()).unwrap_or_default())
                .collect();
            if cur != last_say {
                for (i, s) in cur.iter().enumerate() {
                    if !s.is_empty() && last_say.get(i) != Some(s) {
                        let _ = writeln!(out, "{}", s);
                    }
                }
                last_say = cur;
            }
        }
        if realtime {
            if w.live_threads() == 0 {
                break;
            }
            next_tick += std::time::Duration::from_secs_f64(step_ms / 1000.0);
            let now = std::time::Instant::now();
            if next_tick > now {
                std::thread::sleep(next_tick - now);
            } else {
                next_tick = now;
            }
        } else if !trace && w.live_threads() == 0 && args.frames.is_none() {
            break;
        }
    }
    let _ = out.flush();
    if args.bench {
        eprintln!("frames: {}  elapsed: {:.3} ms", done_frames, started.elapsed().as_secs_f64() * 1000.0);
    }
}
