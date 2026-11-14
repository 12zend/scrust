//! Thread scheduler: a semantic re-implementation of scratch-vm's
//! `Runtime._step` / `Sequencer.stepThreads` / `startHats`.

use crate::world::{assert_world_released, world_ptr, TargetRef, World};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

pub type BoxFut = Pin<Box<dyn Future<Output = ()>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    PromiseWait,
    YieldTick,
    Done,
}

/// What resolves a `PromiseWait` (checked between ticks, like JS promises).
pub enum Wake {
    /// Clock time (ms) at which a `setTimeout` fires.
    Time(f64),
    Flag(Rc<Cell<bool>>),
    /// `say/think for secs`: at `at`, clear the bubble if it is still the same one.
    Bubble { at: f64, target: TargetRef, usage: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HatKind {
    Flag,
    Key,
    SpriteClicked,
    StageClicked,
    Backdrop,
    Greater,
    Broadcast,
    Clone,
    Touching,
}

pub struct Thread {
    pub tid: u64,
    pub target: TargetRef,
    pub script: usize,
    pub fut: Option<BoxFut>,
    pub status: Status,
    pub killed: bool,
    pub wake: Option<Wake>,
    /// Non-warp procedure call chain: (procedure uid, control nesting depth of the call site).
    pub calls: Vec<(u32, u8)>,
    /// Set when a promise block finished in tail position of a loop body:
    /// the loop-end yield of that iteration is skipped (see docs/decisions).
    pub skip_loop_yield: bool,
}

pub struct Sched {
    pub threads: Vec<Thread>,
    /// The list being iterated when `stop all` replaced `threads` mid-pass.
    pub detached: Option<Vec<Thread>>,
    pub in_pass: bool,
    /// Index (in the iterated list) and tid of the thread being stepped.
    pub active: Option<(usize, u64)>,
    pub next_tid: u64,
    pub redraw_requested: bool,
    /// Status a pending poll wants (set by the yield primitives).
    pub pending: Status,
    pub pending_wake: Option<Wake>,
    /// Passes executed in the last tick (for traces).
    pub last_passes: u32,
    /// Busy-wait iterations of the active thread in this step (warp stuck check).
    pub busy: u32,
    pub step_start: f64,
    /// The current tick is abandoned after the active thread yields
    /// (emulates an exception escaping `Runtime._step`).
    pub abort_tick: bool,
}

impl Default for Sched {
    fn default() -> Self {
        Sched {
            threads: Vec::new(),
            detached: None,
            in_pass: false,
            active: None,
            next_tid: 1,
            redraw_requested: false,
            pending: Status::Running,
            pending_wake: None,
            last_passes: 0,
            busy: 0,
            step_start: 0.0,
            abort_tick: false,
        }
    }
}

impl Sched {
    /// The list `stepThreads` is iterating right now.
    fn cur_list(&mut self) -> &mut Vec<Thread> {
        match &mut self.detached {
            Some(d) => d,
            None => &mut self.threads,
        }
    }

    pub fn active_thread(&mut self) -> Option<&mut Thread> {
        let (i, tid) = self.active?;
        let list = self.cur_list();
        match list.get_mut(i) {
            Some(t) if t.tid == tid => Some(t),
            _ => None,
        }
    }
}

impl World {
    fn new_thread(&mut self, target: TargetRef, script: usize) -> Thread {
        let tid = self.sched.next_tid;
        self.sched.next_tid += 1;
        let sprite = self.targets[target.idx as usize].sprite;
        let def = &self.program.sprites[sprite].scripts[script];
        for c in def.creates {
            if !self.created[sprite].contains(c) {
                self.created[sprite].push(*c);
            }
        }
        Thread {
            tid,
            target,
            script,
            fut: Some((def.start)(target)),
            status: Status::Running,
            killed: false,
            wake: None,
            calls: Vec::new(),
            skip_loop_yield: false,
        }
    }

    /// `Runtime.startHats`. Returns the tids of the started threads.
    pub fn start_hats(&mut self, kind: HatKind, field: Option<&str>, msg: usize, only: Option<TargetRef>) -> Vec<u64> {
        let targets: Vec<TargetRef> = match only {
            Some(t) => vec![t],
            None => self.order.iter().rev().copied().collect(),
        };
        let mut new_threads: Vec<u64> = Vec::new();
        let program = self.program;
        for t in targets {
            let Some(tg) = self.target_checked(t) else { continue };
            let sprite = tg.sprite;
            let scripts = &program.sprites[sprite].scripts;
            for (si, sc) in scripts.iter().enumerate() {
                if sc.kind != kind {
                    continue;
                }
                match kind {
                    HatKind::Broadcast => {
                        if sc.msg != msg {
                            continue;
                        }
                    }
                    HatKind::Key | HatKind::Backdrop => {
                        if let Some(f) = field {
                            if sc.field != f {
                                continue;
                            }
                        }
                    }
                    _ => {}
                }
                if sc.restart {
                    if let Some(i) = self.sched.threads.iter().position(|th| th.target == t && th.script == si) {
                        let th = self.new_thread(t, si);
                        new_threads.push(th.tid);
                        self.sched.threads[i] = th;
                        continue;
                    }
                } else if self.sched.threads.iter().any(|th| th.target == t && th.script == si && th.status != Status::Done) {
                    continue;
                }
                let th = self.new_thread(t, si);
                new_threads.push(th.tid);
                self.sched.threads.push(th);
            }
        }
        // Hats are executed right away: edge-activated hats evaluate their
        // predicate here and retire unless it went false -> true.
        if matches!(kind, HatKind::Greater | HatKind::Touching) {
            for &tid in &new_threads {
                let Some((target, script)) = self.sched.threads.iter().find(|th| th.tid == tid).map(|th| (th.target, th.script)) else {
                    continue;
                };
                let sprite = self.targets[target.idx as usize].sprite;
                let Some(pred) = program.sprites[sprite].scripts[script].pred.as_ref() else { continue };
                let value = pred(self, target);
                let tg = &mut self.targets[target.idx as usize];
                if tg.edge_values.len() <= script {
                    tg.edge_values.resize(script + 1, None);
                }
                let old = tg.edge_values[script].replace(value);
                let activated = match old {
                    Some(old) => !old && value,
                    None => value,
                };
                if !activated {
                    if let Some(th) = self.sched.threads.iter_mut().find(|th| th.tid == tid) {
                        th.status = Status::Done;
                        th.fut = None;
                    }
                }
            }
        }
        new_threads
    }

    fn retire(th: &mut Thread) {
        th.status = Status::Done;
    }

    /// `Runtime.stopForTarget`.
    pub fn stop_for_target(&mut self, target: TargetRef, except_active: bool) {
        let active = self.sched.active.map(|a| a.1);
        for th in self.sched.threads.iter_mut().chain(self.sched.detached.iter_mut().flatten()) {
            if except_active && Some(th.tid) == active {
                continue;
            }
            if th.target == target {
                th.killed = true;
                Self::retire(th);
            }
        }
    }

    /// `Runtime.stopAll`.
    pub fn stop_all(&mut self) {
        let all: Vec<TargetRef> = self.seq.clone();
        for t in all {
            self.clear_effects(t);
            if !self.targets[t.idx as usize].is_original {
                self.dispose_target(t);
            }
        }
        self.on_stop_all();
        if let Some(th) = self.sched.active_thread() {
            th.killed = true;
            Self::retire(th);
        }
        if self.sched.in_pass && self.sched.detached.is_none() {
            // The running pass keeps iterating the old array.
            self.sched.detached = Some(std::mem::take(&mut self.sched.threads));
        } else {
            self.sched.threads.clear();
        }
    }

    /// `Runtime.greenFlag`.
    pub fn green_flag(&mut self) {
        self.stop_all();
        self.timer_start = self.current_msecs;
        let all: Vec<TargetRef> = self.seq.clone();
        for t in all {
            self.targets[t.idx as usize].edge_values.clear();
            self.clear_effects(t);
        }
        self.start_hats(HatKind::Flag, None, 0, None);
    }

    /// Is the thread still in `runtime.threads`?
    pub fn thread_in_list(&self, tid: u64) -> bool {
        self.sched.threads.iter().any(|t| t.tid == tid)
    }

    /// `Runtime.isWaitingThread`.
    pub fn thread_is_waiting(&self, tid: u64) -> bool {
        match self.sched.threads.iter().find(|t| t.tid == tid) {
            Some(t) => matches!(t.status, Status::PromiseWait | Status::YieldTick | Status::Done),
            None => true,
        }
    }

    /// The "wait" half of `broadcast and wait` / `switch backdrop and wait`:
    /// returns None when the wait is over, otherwise how to yield.
    pub fn wait_threads_status(&self, tids: &[u64]) -> Option<Status> {
        if !tids.iter().any(|&t| self.thread_in_list(t)) {
            return None;
        }
        if tids.iter().all(|&t| self.thread_is_waiting(t)) {
            Some(Status::YieldTick)
        } else {
            Some(Status::Running)
        }
    }

    /// Number of live (not done, not killed) threads.
    pub fn live_threads(&self) -> usize {
        self.sched.threads.iter().filter(|t| t.status != Status::Done && !t.killed).count()
    }

    // --- procedure call bookkeeping (non-warp recursion yields) ---------------

    /// `Thread.isRecursiveCall` for a call to `callee` made from a call site
    /// nested inside `depth` control blocks; then records the call.
    pub fn enter_call(&mut self, callee: u32, depth: u8) -> bool {
        let Some(th) = self.sched.active_thread() else { return false };
        // scratch-vm inspects the 6 stack levels below the call block.
        let mut remaining: i32 = 6 - depth as i32;
        let mut recursive = false;
        for &(uid, d) in th.calls.iter().rev() {
            if remaining <= 0 {
                break;
            }
            if uid == callee {
                recursive = true;
                break;
            }
            remaining -= 1 + d as i32;
        }
        th.calls.push((callee, depth));
        recursive
    }

    pub fn exit_call(&mut self) {
        if let Some(th) = self.sched.active_thread() {
            th.calls.pop();
        }
    }

    /// scratch-vm's `switch backdrop to () and wait` throws a TypeError right
    /// after starting hats (it reads the stack frame of the wrong thread), and
    /// the exception unwinds the whole `_step`: the rest of the tick is lost.
    pub fn abort_tick(&mut self) {
        self.sched.abort_tick = true;
    }

    /// A promise block completed in tail position of a loop body.
    pub fn set_skip_loop_yield(&mut self, v: bool) {
        if let Some(th) = self.sched.active_thread() {
            th.skip_loop_yield = v;
        }
    }
}

// ---------------------------------------------------------------------------
// Yield primitives used by generated code.

/// Suspends once with the given status.
pub struct Suspend {
    status: Status,
    wake: Option<Wake>,
    done: bool,
}

impl Future for Suspend {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.done {
            return Poll::Ready(());
        }
        self.done = true;
        assert_world_released();
        // SAFETY: threads are only polled by `step`, which has published the
        // world pointer and holds no reference to it across the poll.
        let w = unsafe { &mut *world_ptr() };
        w.sched.pending = self.status;
        w.sched.pending_wake = self.wake.take();
        Poll::Pending
    }
}

/// `util.yield()`: resume in the next pass (possibly in the same frame).
#[inline]
pub fn yield_now() -> Suspend {
    Suspend { status: Status::Running, wake: None, done: false }
}

/// `util.yieldTick()`: resume in the next frame.
#[inline]
pub fn yield_tick() -> Suspend {
    Suspend { status: Status::YieldTick, wake: None, done: false }
}

/// Yield with a status computed by `wait_threads_status`.
#[inline]
pub fn yield_status(status: Status) -> Suspend {
    Suspend { status, wake: None, done: false }
}

/// Promise wait: resume once `wake` resolves (checked between frames).
#[inline]
pub fn wait_external(wake: Wake) -> Suspend {
    Suspend { status: Status::PromiseWait, wake: Some(wake), done: false }
}

/// End-of-iteration yield of a non-warp loop.
pub struct LoopYield {
    done: bool,
}

impl Future for LoopYield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.done {
            return Poll::Ready(());
        }
        self.done = true;
        assert_world_released();
        // SAFETY: see `Suspend::poll`.
        let w = unsafe { &mut *world_ptr() };
        if let Some(th) = w.sched.active_thread() {
            if th.skip_loop_yield {
                th.skip_loop_yield = false;
                return Poll::Ready(());
            }
        }
        w.sched.pending = Status::Running;
        Poll::Pending
    }
}

#[inline]
pub fn yield_loop() -> LoopYield {
    LoopYield { done: false }
}

/// Terminates the current thread from any call depth: never resumes.
pub struct Die;
impl Future for Die {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        assert_world_released();
        // SAFETY: see `Suspend::poll`.
        let w = unsafe { &mut *world_ptr() };
        w.sched.pending = Status::Done;
        Poll::Pending
    }
}

#[inline]
pub fn die() -> Die {
    Die
}

// ---------------------------------------------------------------------------
// The tick loop. Works through a raw pointer so that no `&mut World` is alive
// while a thread future (which re-derives its own) is being polled.

/// `Runtime._step` minus rendering.
///
/// # Safety
/// `wp` must point to a live `World` that is not otherwise borrowed.
pub unsafe fn step(wp: *mut World) {
    crate::world::set_world_ptr(wp);
    {
        let w = &mut *wp;
        w.advance_clock();
        w.resolve_promises();
        w.sched.threads.retain(|t| !t.killed);
        w.free_disposed();
        // `_hats` order: event_whentouchingobject precedes event_whengreaterthan.
        w.start_hats(HatKind::Touching, None, 0, None);
        w.start_hats(HatKind::Greater, None, 0, None);
        w.sched.redraw_requested = false;
    }
    step_threads(wp);
    {
        let w = &mut *wp;
        w.with_renderer(|r, w| r.frame_drawn(w));
    }
    (*wp).frame += 1;
}

unsafe fn step_threads(wp: *mut World) {
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let (work_time, max_passes) = {
        let w = &mut *wp;
        w.current_msecs = w.clock_ms();
        w.sched.step_start = w.real_ms();
        (0.75 * w.step_time, w.options.max_passes)
    };
    let mut num_active = usize::MAX;
    let mut ran_first_tick = false;
    let mut passes: u32 = 0;
    (*wp).sched.abort_tick = false;
    // `_pushMonitors` queues one thread per visible monitor before stepping;
    // they finish in the first pass and change nothing but keep the thread
    // list non-empty for it.
    let monitor_threads = (*wp).monitor_visible.iter().any(|v| *v);
    'tick: loop {
        {
            let w = &mut *wp;
            let time_ok = match max_passes {
                Some(n) => passes < n,
                None => w.real_ms() - w.sched.step_start < work_time,
            };
            if (w.sched.threads.is_empty() && !(monitor_threads && passes == 0)) || num_active == 0 || !time_ok || !(w.options.turbo || !w.sched.redraw_requested) {
                break;
            }
            w.sched.in_pass = true;
        }
        num_active = 0;
        let mut stopped = false;
        let mut i = 0;
        loop {
            // Decide what to do with thread i.
            let job = {
                let w = &mut *wp;
                let list = w.sched.cur_list();
                if i >= list.len() {
                    break;
                }
                let th = &mut list[i];
                if th.status == Status::Done {
                    stopped = true;
                    i += 1;
                    continue;
                }
                if th.status == Status::YieldTick && !ran_first_tick {
                    th.status = Status::Running;
                }
                if th.status == Status::Running {
                    let tid = th.tid;
                    let fut = th.fut.take();
                    w.sched.active = Some((i, tid));
                    w.sched.pending = Status::Running;
                    w.sched.pending_wake = None;
                    w.sched.busy = 0;
                    Some((tid, fut))
                } else {
                    None
                }
            };
            let mut final_status = None;
            if let Some((tid, fut)) = job {
                let mut result = Status::Done;
                let mut fut = fut;
                if let Some(f) = fut.as_mut() {
                    result = match f.as_mut().poll(&mut cx) {
                        Poll::Ready(()) => Status::Done,
                        Poll::Pending => (*wp).sched.pending,
                    };
                }
                let w = &mut *wp;
                let wake = w.sched.pending_wake.take();
                w.sched.active = None;
                let list = w.sched.cur_list();
                let th = &mut list[i];
                if th.tid == tid {
                    if th.status == Status::Done {
                        // Retired during its own step (stop all, delete this clone, ...).
                    } else {
                        th.status = result;
                        if result == Status::Done {
                            // Dropped below, outside of the list borrow.
                        } else {
                            th.fut = fut.take();
                            th.wake = wake;
                        }
                    }
                    final_status = Some(th.status);
                } else {
                    // Restarted in place while running: the old thread ran to
                    // its yield and is never resumed.
                    final_status = Some(result);
                }
                drop(fut);
                if w.sched.abort_tick {
                    w.sched.abort_tick = false;
                    w.sched.in_pass = false;
                    if let Some(old) = w.sched.detached.take() {
                        drop(old);
                    }
                    passes += 1;
                    break 'tick;
                }
            }
            let status = match final_status {
                Some(s) => s,
                None => (&mut *wp).sched.cur_list()[i].status,
            };
            if status == Status::Running {
                num_active += 1;
            }
            if status == Status::Done {
                stopped = true;
            }
            i += 1;
        }
        ran_first_tick = true;
        passes += 1;
        let w = &mut *wp;
        w.sched.in_pass = false;
        let old = w.sched.detached.take();
        drop(old);
        if stopped {
            w.sched.threads.retain(|t| t.status != Status::Done);
        }
    }
    (*wp).sched.last_passes = passes;
}
