//! Virtual audio: no output, but sounds take as long as they really are.
//! Gives `play sound until done` its real duration in headless and
//! deterministic runs (`--virtual-audio`).

use crate::world::{Audio, ProgramDef, TargetRef};
use std::cell::Cell;
use std::rc::Rc;

struct Playing {
    target: TargetRef,
    sound: usize,
    /// Remaining source time in ms.
    remaining: f64,
    rate: f64,
    done: Rc<Cell<bool>>,
}

pub struct VirtualAudio {
    program: &'static ProgramDef,
    playing: Vec<Playing>,
    last: Option<f64>,
}

impl VirtualAudio {
    pub fn new(program: &'static ProgramDef) -> VirtualAudio {
        VirtualAudio { program, playing: Vec::new(), last: None }
    }
}

/// Duration in ms of a sound: project.json metadata, else the WAV header.
pub fn sound_duration_ms(def: &crate::world::SoundDef) -> f64 {
    if def.rate > 0.0 && def.sample_count > 0.0 {
        return def.sample_count / def.rate * 1000.0;
    }
    let d = def.data;
    if d.len() >= 44 && &d[0..4] == b"RIFF" {
        let byte_rate = u32::from_le_bytes([d[28], d[29], d[30], d[31]]) as f64;
        if byte_rate > 0.0 {
            return (d.len() as f64 - 44.0) / byte_rate * 1000.0;
        }
    }
    0.0
}

impl Audio for VirtualAudio {
    fn play(&mut self, sprite: usize, sound: usize, target: TargetRef, _volume: f64, pitch: f64, _pan: f64) -> Option<Rc<Cell<bool>>> {
        self.stop_matching(|p| p.target == target && p.sound == sound);
        let def = self.program.sprites.get(sprite)?.sounds.get(sound)?;
        let done = Rc::new(Cell::new(false));
        self.playing.push(Playing { target, sound, remaining: sound_duration_ms(def), rate: 2f64.powf(pitch / 120.0), done: done.clone() });
        Some(done)
    }
    fn stop_all(&mut self) {
        self.stop_matching(|_| true);
    }
    fn stop_target(&mut self, target: TargetRef) {
        self.stop_matching(|p| p.target == target);
    }
    fn update_target(&mut self, target: TargetRef, _volume: f64, pitch: f64, _pan: f64) {
        for p in self.playing.iter_mut().filter(|p| p.target == target) {
            p.rate = 2f64.powf(pitch / 120.0);
        }
    }
    fn loudness(&mut self) -> f64 {
        -1.0
    }
    fn tick(&mut self, now_ms: f64) {
        let dt = self.last.map(|l| now_ms - l).unwrap_or(0.0).max(0.0);
        self.last = Some(now_ms);
        for p in self.playing.iter_mut() {
            p.remaining -= dt * p.rate;
        }
        self.playing.retain(|p| {
            if p.remaining <= 0.0 {
                p.done.set(true);
                false
            } else {
                true
            }
        });
    }
}

impl VirtualAudio {
    fn stop_matching(&mut self, f: impl Fn(&Playing) -> bool) {
        self.playing.retain(|p| {
            if f(p) {
                p.done.set(true);
                false
            } else {
                true
            }
        });
    }
}
