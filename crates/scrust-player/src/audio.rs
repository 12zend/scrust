//! Audio backend on rodio: per-sound sinks with live volume, pitch and pan.

use rodio::source::Source;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use scrust_runtime::{Audio, ProgramDef, TargetRef};
use std::cell::Cell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Stereo adapter applying live left/right gains (the pan effect).
struct Pan<S: Source<Item = f32>> {
    inner: S,
    channels: u16,
    gains: Arc<(AtomicU32, AtomicU32)>,
    pending: Option<f32>,
    left_next: bool,
}

impl<S: Source<Item = f32>> Iterator for Pan<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let gain = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        if let Some(r) = self.pending.take() {
            self.left_next = true;
            return Some(r * gain(&self.gains.1));
        }
        let l = self.inner.next()?;
        let r = if self.channels >= 2 {
            let r = self.inner.next().unwrap_or(l);
            // Drop any extra channels.
            for _ in 2..self.channels {
                self.inner.next();
            }
            r
        } else {
            l
        };
        self.pending = Some(r);
        self.left_next = false;
        Some(l * gain(&self.gains.0))
    }
}

impl<S: Source<Item = f32>> Source for Pan<S> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

struct Playing {
    sink: Sink,
    target: TargetRef,
    sound: usize,
    done: Rc<Cell<bool>>,
    gains: Arc<(AtomicU32, AtomicU32)>,
}

pub struct RodioAudio {
    program: &'static ProgramDef,
    _stream: OutputStream,
    handle: OutputStreamHandle,
    playing: Vec<Playing>,
}

/// scratch-audio's pan effect: equal-power-ish linear gains.
fn pan_gains(pan: f64) -> (f32, f32) {
    let p = ((pan + 100.0) / 200.0).clamp(0.0, 1.0);
    let left = (p * std::f64::consts::FRAC_PI_2).cos();
    let right = (p * std::f64::consts::FRAC_PI_2).sin();
    // Centre is unity gain on both channels.
    let norm = std::f64::consts::FRAC_1_SQRT_2;
    ((left / norm).min(1.0) as f32, (right / norm).min(1.0) as f32)
}

/// scratch-audio's pitch effect: playback rate 2^(pitch / 120).
fn pitch_rate(pitch: f64) -> f32 {
    2f64.powf(pitch / 120.0) as f32
}

impl RodioAudio {
    pub fn new(program: &'static ProgramDef) -> Option<RodioAudio> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(RodioAudio { program, _stream: stream, handle, playing: Vec::new() })
    }
}

impl Audio for RodioAudio {
    fn play(&mut self, sprite: usize, sound: usize, target: TargetRef, volume: f64, pitch: f64, pan: f64) -> Option<Rc<Cell<bool>>> {
        // Starting a sound that is already playing on this target restarts it.
        self.playing.retain(|p| {
            if p.target == target && p.sound == sound {
                p.sink.stop();
                p.done.set(true);
                false
            } else {
                true
            }
        });
        let def = self.program.sprites.get(sprite)?.sounds.get(sound)?;
        let source = Decoder::new(Cursor::new(def.data)).ok()?;
        let sink = Sink::try_new(&self.handle).ok()?;
        let (l, r) = pan_gains(pan);
        let gains = Arc::new((AtomicU32::new(l.to_bits()), AtomicU32::new(r.to_bits())));
        let src = source.convert_samples::<f32>();
        let channels = src.channels();
        sink.set_volume((volume / 100.0) as f32);
        sink.set_speed(pitch_rate(pitch));
        sink.append(Pan { inner: src, channels, gains: gains.clone(), pending: None, left_next: true });
        let done = Rc::new(Cell::new(false));
        self.playing.push(Playing { sink, target, sound, done: done.clone(), gains });
        Some(done)
    }

    fn stop_all(&mut self) {
        for p in self.playing.drain(..) {
            p.sink.stop();
            p.done.set(true);
        }
    }

    fn stop_target(&mut self, target: TargetRef) {
        self.playing.retain(|p| {
            if p.target == target {
                p.sink.stop();
                p.done.set(true);
                false
            } else {
                true
            }
        });
    }

    fn update_target(&mut self, target: TargetRef, volume: f64, pitch: f64, pan: f64) {
        let (l, r) = pan_gains(pan);
        for p in self.playing.iter().filter(|p| p.target == target) {
            p.sink.set_volume((volume / 100.0) as f32);
            p.sink.set_speed(pitch_rate(pitch));
            p.gains.0.store(l.to_bits(), Ordering::Relaxed);
            p.gains.1.store(r.to_bits(), Ordering::Relaxed);
        }
    }

    fn loudness(&mut self) -> f64 {
        // No microphone input.
        -1.0
    }

    fn tick(&mut self, _now_ms: f64) {
        self.playing.retain(|p| {
            if p.sink.empty() {
                p.done.set(true);
                false
            } else {
                true
            }
        });
    }
}
