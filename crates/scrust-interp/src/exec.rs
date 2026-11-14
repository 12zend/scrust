//! The interpreter. Each function corresponds to what `scrust-codegen`
//! emits for the same IR node with every storage representation `Value`
//! (`--no-unbox`); comments name the differences that matter.
//!
//! Like generated code it has a synchronous half (`&mut World` passed in, no
//! suspension) and an asynchronous half (thread futures that reach the world
//! through `world()` and never hold it across an `.await`). A statement runs
//! in the asynchronous half only when it may suspend (`Interp::may_yield`),
//! exactly where the code generator emits `.await`s.

// `!(n >= 0.0)` is meant: NaN must end the loop, as in the generated code.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{interp, ILocals};
use scrust_ir::{BroadcastRef, Cmd, Expr, Hat, ListId, Op, ProcId, Rep, Stmt, VarId, A, K};
use scrust_runtime::prelude::*;
use std::future::Future;
use std::pin::Pin;

/// What follows a statement list (`Tail` in the code generator).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    No,
    Loop,
    Fn,
}

/// Did `stop this script` run (the function returns)?
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Next,
    Stop,
}

/// The function being run: a script or a procedure variant.
struct Frame {
    me: TargetRef,
    ti: usize,
    proc: Option<ProcId>,
    warp: bool,
    /// The generated function would be `async` (it may suspend).
    is_async: bool,
    args: Vec<Value>,
}

type Fut<'a> = Pin<Box<dyn Future<Output = Flow> + 'a>>;

fn undefined() -> Value {
    Value::str("undefined")
}

// ------------------------------------------------------------------ entry points

pub(crate) fn start_script(ti: usize, si: usize, me: TargetRef) -> BoxFut {
    let body = &interp().program.targets[ti].scripts[si].body;
    Box::pin(async move {
        let fr = Frame { me, ti, proc: None, warp: false, is_async: true, args: Vec::new() };
        block(&fr, body, Tail::Fn).await;
    })
}

pub(crate) fn hat_predicate(w: &mut World, ti: usize, si: usize, me: TargetRef) -> bool {
    let fr = Frame { me, ti, proc: None, warp: false, is_async: false, args: Vec::new() };
    match &interp().program.targets[ti].scripts[si].hat {
        Hat::GreaterThan { menu, value } => {
            let v = num(w, &fr, value);
            match menu.as_str() {
                "timer" => w.timer_value() > v,
                "loudness" => w.loudness(me) > v,
                _ => false,
            }
        }
        Hat::TouchingObject(obj) => {
            let v = eval(w, &fr, obj);
            w.touching_object(me, v)
        }
        _ => false,
    }
}

fn proc_is_async(id: ProcId, warp: bool) -> bool {
    interp().an.yields.get(&(id, warp)).copied().unwrap_or(true)
}

// ------------------------------------------------------------------ storage

fn locals_of<'w>(w: &'w mut World, fr: &Frame, target: usize, of_original: bool) -> &'w mut ILocals {
    if target == 0 {
        w.globals::<ILocals>()
    } else if of_original || target != fr.ti {
        w.locals_of::<ILocals>(target)
    } else {
        w.locals::<ILocals>(fr.me)
    }
}

fn var<'w>(w: &'w mut World, fr: &Frame, v: VarId, of_original: bool) -> &'w mut Value {
    &mut locals_of(w, fr, v.target, of_original).vars[v.index]
}

fn list<'w>(w: &'w mut World, fr: &Frame, l: ListId) -> &'w mut List<Value> {
    // Lists are only ever read from the stage or the running target.
    let s = if l.target == 0 { w.globals::<ILocals>() } else { w.locals::<ILocals>(fr.me) };
    &mut s.lists[l.index]
}

// ------------------------------------------------------------------ expressions

/// The raw value (`E::val`): an empty slot is the string "undefined".
fn eval(w: &mut World, fr: &Frame, e: &Expr) -> Value {
    match e {
        Expr::Lit(v) => v.clone(),
        Expr::Missing => undefined(),
        Expr::Var(v) => var(w, fr, *v, false).clone(),
        Expr::VarOf(v) => var(w, fr, *v, true).clone(),
        Expr::Arg(i) => match fr.proc {
            Some(_) => fr.args.get(*i).cloned().unwrap_or_else(undefined),
            None => Value::Num(0.0),
        },
        Expr::ListContents(l) => Value::Str(list(w, fr, *l).contents()),
        Expr::ListLen(l) => Value::Num(list(w, fr, *l).len() as f64),
        Expr::ListItem(l, idx) => {
            let i = eval(w, fr, idx);
            let n = list(w, fr, *l).len();
            let ix = w.list_index(&i, n, false);
            list(w, fr, *l).item(ix)
        }
        Expr::ListIndexOf(l, item) => {
            let v = eval(w, fr, item);
            Value::Num(list(w, fr, *l).index_of(&v))
        }
        Expr::ListContains(l, item) => {
            let v = eval(w, fr, item);
            Value::Bool(list(w, fr, *l).contains(&v))
        }
        Expr::Rep(rep, args) => reporter(w, fr, *rep, args),
        Expr::Op(op, args, order) => operator(w, fr, *op, args, order.as_deref()),
    }
}

/// `Cast.toNumber` (an empty slot is 0, as "undefined" casts).
fn num(w: &mut World, fr: &Frame, e: &Expr) -> f64 {
    eval(w, fr, e).to_num()
}

/// `Cast.toString`
fn string(w: &mut World, fr: &Frame, e: &Expr) -> JsStr {
    eval(w, fr, e).to_str()
}

/// `Cast.toBoolean`; an empty boolean slot is false (not "undefined").
fn boolean(w: &mut World, fr: &Frame, e: &Expr) -> bool {
    match e {
        Expr::Missing => false,
        _ => eval(w, fr, e).to_bool(),
    }
}

fn operator(w: &mut World, fr: &Frame, op: Op, args: &[Expr], order: Option<&[usize]>) -> Value {
    // TurboWarp folds `pick random` between two equal literal inputs without
    // drawing a random number.
    if let (Op::Random, [Expr::Lit(a), Expr::Lit(b)]) = (op, args) {
        if a.to_num() == b.to_num() {
            return Value::Num(a.to_num());
        }
    }
    if let Some(order) = order {
        // A recorded evaluation order: every operand is evaluated first, into
        // raw values (no short-circuit, empty slots become "undefined").
        let mut vals: Vec<Value> = vec![Value::Num(0.0); args.len()];
        for &i in order {
            vals[i] = eval(w, fr, &args[i]);
        }
        return apply(w, op, &vals);
    }
    match op {
        // JavaScript's `&&` / `||`: the right operand only when needed.
        Op::And => Value::Bool(boolean(w, fr, &args[0]) && boolean(w, fr, &args[1])),
        Op::Or => Value::Bool(boolean(w, fr, &args[0]) || boolean(w, fr, &args[1])),
        Op::Not => Value::Bool(!boolean(w, fr, &args[0])),
        _ => {
            let vals: Vec<Value> = args.iter().map(|a| eval(w, fr, a)).collect();
            apply(w, op, &vals)
        }
    }
}

fn apply(w: &mut World, op: Op, v: &[Value]) -> Value {
    let n = |i: usize| v[i].to_num();
    let s = |i: usize| v[i].to_str();
    match op {
        Op::Add => Value::Num(n(0) + n(1)),
        Op::Sub => Value::Num(n(0) - n(1)),
        Op::Mul => Value::Num(n(0) * n(1)),
        Op::Div => Value::Num(n(0) / n(1)),
        Op::Mod => Value::Num(ops::js_mod(n(0), n(1))),
        Op::Round => Value::Num(js_round(n(0))),
        Op::Lt => Value::Bool(compare(&v[0], &v[1]) < 0.0),
        Op::Eq => Value::Bool(compare(&v[0], &v[1]) == 0.0),
        Op::Gt => Value::Bool(compare(&v[0], &v[1]) > 0.0),
        Op::And => Value::Bool(v[0].to_bool() && v[1].to_bool()),
        Op::Or => Value::Bool(v[0].to_bool() || v[1].to_bool()),
        Op::Not => Value::Bool(!v[0].to_bool()),
        Op::Join => Value::Str(ops::join(&s(0), &s(1))),
        Op::LetterOf => Value::Str(ops::letter_of(n(0), &s(1))),
        Op::Length => Value::Num(s(0).len() as f64),
        Op::Contains => Value::Bool(ops::contains(&s(0), &s(1))),
        Op::Random => Value::Num(w.random_values(&v[0], &v[1])),
        Op::Math(m) => Value::Num(m.apply(n(0))),
        Op::MathDyn => Value::Num(ops::mathop(&s(0), n(1))),
    }
}

/// An argument of a table-driven block, after the cast its slot asks for.
enum Arg {
    Field(String),
    V(Value),
}

impl Arg {
    fn f(&self) -> &str {
        match self {
            Arg::Field(s) => s,
            Arg::V(_) => "",
        }
    }
    fn v(&self) -> Value {
        match self {
            Arg::V(v) => v.clone(),
            Arg::Field(s) => Value::str(s),
        }
    }
    fn n(&self) -> f64 {
        self.v().to_num()
    }
    fn s(&self) -> JsStr {
        self.v().to_str()
    }
}

fn arg(w: &mut World, fr: &Frame, spec: &A, e: &Expr) -> Arg {
    match spec {
        A::Field(_) => Arg::Field(match e {
            Expr::Lit(v) => v.to_str().to_rust_string(),
            _ => String::new(),
        }),
        A::In(_, k) => Arg::V(match k {
            K::Num => Value::Num(num(w, fr, e)),
            K::Str => Value::Str(string(w, fr, e)),
            K::Bool => Value::Bool(boolean(w, fr, e)),
            K::Raw | K::RawKind => eval(w, fr, e),
        }),
    }
}

fn reporter(w: &mut World, fr: &Frame, rep: Rep, args: &[Expr]) -> Value {
    let a: Vec<Arg> = rep.args().iter().zip(args).map(|(spec, e)| arg(w, fr, spec, e)).collect();
    let me = fr.me;
    let n = Value::Num;
    let s = Value::Str;
    let b = Value::Bool;
    match rep {
        Rep::XPosition => n(w.x_position(me)),
        Rep::YPosition => n(w.y_position(me)),
        Rep::Direction => n(w.direction(me)),
        Rep::Size => n(w.size(me)),
        Rep::Volume => n(w.volume(me)),
        Rep::TouchingObject => b(w.touching_object(me, a[0].v())),
        Rep::TouchingColor => b(w.touching_color(me, a[0].v())),
        Rep::ColorTouchingColor => b(w.color_touching_color(me, a[0].v(), a[1].v())),
        Rep::DistanceTo => n(w.distance_to(me, a[0].v())),
        Rep::Answer => s(w.answer(me)),
        Rep::KeyPressed => b(w.key_pressed(me, a[0].v())),
        Rep::MouseDown => b(w.mouse_down(me)),
        Rep::MouseX => n(w.mouse_x(me)),
        Rep::MouseY => n(w.mouse_y(me)),
        Rep::Loudness => n(w.loudness(me)),
        Rep::Loud => b(w.loud(me)),
        Rep::Timer => n(w.timer(me)),
        Rep::Current => n(w.current(me, a[0].f())),
        Rep::DaysSince2000 => n(w.days_since_2000(me)),
        Rep::Username => s(w.username(me)),
        Rep::MusicTempo => n(w.music_tempo(me)),
        Rep::ViewerLanguage => s(w.viewer_language(me)),
        Rep::Translate => s(w.translate(me, a[0].s(), a[1].s())),
        Rep::Counter => n(w.counter(me)),
        Rep::CostumeNumber => n(w.costume_number(me)),
        Rep::CostumeName => s(w.costume_name(me)),
        Rep::BackdropNumber => n(w.backdrop_number(me)),
        Rep::BackdropName => s(w.backdrop_name(me)),
        Rep::AttrOf => w.attr_of(me, a[0].f(), a[1].v()),
    }
}

/// A table-driven command; promise commands return their wake-up.
fn command(w: &mut World, fr: &Frame, cmd: Cmd, args: &[Expr], order: &[usize]) -> Option<Wake> {
    let specs = cmd.args();
    let mut slots: Vec<Option<Arg>> = (0..args.len()).map(|_| None).collect();
    for &i in order {
        slots[i] = Some(arg(w, fr, &specs[i], &args[i]));
    }
    // Fields are not inputs and take no part in the evaluation order.
    for (i, spec) in specs.iter().enumerate().take(args.len()) {
        if slots[i].is_none() {
            slots[i] = Some(arg(w, fr, spec, &args[i]));
        }
    }
    let a: Vec<Arg> = slots.into_iter().map(|s| s.unwrap_or(Arg::Field(String::new()))).collect();
    let me = fr.me;
    match cmd {
        Cmd::MoveSteps => w.move_steps(me, a[0].n()),
        Cmd::TurnRight => w.turn_right(me, a[0].n()),
        Cmd::TurnLeft => w.turn_left(me, a[0].n()),
        Cmd::GoTo => w.go_to(me, a[0].v()),
        Cmd::GoToXY => w.go_to_xy(me, a[0].n(), a[1].n()),
        Cmd::PointInDirection => w.point_in_direction(me, a[0].n()),
        Cmd::PointTowards => w.point_towards(me, a[0].v()),
        Cmd::ChangeX => w.change_x(me, a[0].n()),
        Cmd::SetX => w.set_x(me, a[0].n()),
        Cmd::ChangeY => w.change_y(me, a[0].n()),
        Cmd::SetY => w.set_y(me, a[0].n()),
        Cmd::IfOnEdgeBounce => w.if_on_edge_bounce(me),
        Cmd::SetRotationStyle => w.set_rotation_style(me, a[0].f()),
        Cmd::Say => w.say(me, a[0].v()),
        Cmd::Think => w.think(me, a[0].v()),
        Cmd::Show => w.show(me),
        Cmd::Hide => w.hide(me),
        Cmd::SwitchCostume => w.switch_costume(me, a[0].v()),
        Cmd::NextCostume => w.next_costume(me),
        Cmd::SwitchBackdrop => w.switch_backdrop(me, a[0].v()),
        Cmd::NextBackdrop => w.next_backdrop(me),
        Cmd::ChangeEffect => w.change_effect(me, a[0].f(), a[1].n()),
        Cmd::SetEffect => w.set_effect(me, a[0].f(), a[1].n()),
        Cmd::ClearEffects => w.clear_effects(me),
        Cmd::ChangeSize => w.change_size(me, a[0].n()),
        Cmd::SetSize => w.set_size(me, a[0].n()),
        Cmd::GoToFrontBack => w.go_to_front_back(me, a[0].f()),
        Cmd::GoLayers => w.go_layers(me, a[0].f(), a[1].n()),
        Cmd::HideAllSprites => w.noop0(me),
        Cmd::ChangeStretch | Cmd::SetStretch | Cmd::MusicSetInstrument | Cmd::MusicMidiSetInstrument => w.noop1(me, a[0].v()),
        Cmd::SoundPlay => w.sound_play(me, a[0].v()),
        Cmd::SoundStopAll => w.sound_stop_all(me),
        Cmd::SoundChangeEffect => return w.sound_change_effect(me, a[0].f(), a[1].n()),
        Cmd::SoundSetEffect => return w.sound_set_effect(me, a[0].f(), a[1].n()),
        Cmd::SoundClearEffects => w.sound_clear_effects(me),
        Cmd::SoundChangeVolume => return w.sound_change_volume(me, a[0].n()),
        Cmd::SoundSetVolume => return w.sound_set_volume(me, a[0].n()),
        Cmd::CreateClone => w.create_clone(me, a[0].s()),
        Cmd::ClearCounter => w.clear_counter(me),
        Cmd::IncrCounter => w.incr_counter(me),
        Cmd::SetDragMode => w.set_drag_mode(me, a[0].f()),
        Cmd::ResetTimer => w.reset_timer(me),
        Cmd::PenClear => w.pen_clear(me),
        Cmd::PenStamp => w.pen_stamp(me),
        Cmd::PenDown => w.pen_down(me),
        Cmd::PenUp => w.pen_up(me),
        Cmd::PenSetColor => w.pen_set_color(me, a[0].v()),
        Cmd::PenChangeParam => w.pen_change_param(me, a[0].s(), a[1].n()),
        Cmd::PenSetParam => w.pen_set_param(me, a[0].s(), a[1].n()),
        Cmd::PenChangeSize => w.pen_change_size(me, a[0].n()),
        Cmd::PenSetSize => w.pen_set_size(me, a[0].n()),
        Cmd::PenSetShade => w.pen_set_shade(me, a[0].n()),
        Cmd::PenChangeShade => w.pen_change_shade(me, a[0].n()),
        Cmd::PenSetHue => w.pen_set_hue(me, a[0].n()),
        Cmd::PenChangeHue => w.pen_change_hue(me, a[0].n()),
        Cmd::MusicSetTempo => w.music_set_tempo(me, a[0].n()),
        Cmd::MusicChangeTempo => w.music_change_tempo(me, a[0].n()),
    }
    None
}

/// Evaluate the inputs whose evaluation is observable (random numbers).
fn discard(w: &mut World, fr: &Frame, args: &[Expr]) {
    for e in args {
        if e.is_impure() {
            let _ = eval(w, fr, e);
        }
    }
}

fn broadcast(w: &mut World, fr: &Frame, b: &BroadcastRef) -> Vec<u64> {
    match b {
        BroadcastRef::Static(m) => w.broadcast(*m),
        BroadcastRef::Nothing => Vec::new(),
        BroadcastRef::Dynamic(e) => {
            let name = string(w, fr, e);
            w.broadcast_dynamic(name)
        }
    }
}

/// Arguments of a procedure call, evaluated in `order`.
fn call_args(w: &mut World, fr: &Frame, args: &[Expr], order: &[usize]) -> Vec<Value> {
    let mut vals: Vec<Value> = vec![Value::Num(0.0); args.len()];
    for &i in order {
        vals[i] = eval(w, fr, &args[i]);
    }
    vals
}

fn proc_frame(fr: &Frame, proc: ProcId, warp: bool, is_async: bool, args: Vec<Value>) -> Frame {
    Frame { me: fr.me, ti: proc.target, proc: Some(proc), warp, is_async, args }
}

// ------------------------------------------------------------------ synchronous statements

fn block_sync(w: &mut World, fr: &Frame, body: &[Stmt]) -> Flow {
    for s in body {
        if stmt_sync(w, fr, s) == Flow::Stop {
            return Flow::Stop;
        }
    }
    Flow::Next
}

/// A statement that cannot suspend (or runs in a function that never does).
fn stmt_sync(w: &mut World, fr: &Frame, s: &Stmt) -> Flow {
    let me = fr.me;
    match s {
        Stmt::SetVar(v, e) => {
            let x = eval(w, fr, e);
            *var(w, fr, *v, false) = x;
        }
        Stmt::ChangeVar(v, e) => {
            let d = num(w, fr, e);
            let p = var(w, fr, *v, false);
            *p = Value::Num(p.to_num() + d);
        }
        Stmt::ShowVar(v, show) => w.set_monitor_visible(v.target, false, v.index, *show),
        Stmt::ShowList(l, show) => w.set_monitor_visible(l.target, true, l.index, *show),
        Stmt::ListAdd(l, e) => {
            let x = eval(w, fr, e);
            list(w, fr, *l).add(x);
        }
        Stmt::ListDelete(l, idx) => {
            let i = eval(w, fr, idx);
            let n = list(w, fr, *l).len();
            let ix = w.list_index(&i, n, true);
            list(w, fr, *l).delete(ix);
        }
        Stmt::ListDeleteAll(l) => list(w, fr, *l).delete_all(),
        Stmt::ListInsert(l, idx, item) => {
            // The item first, then the index.
            let x = eval(w, fr, item);
            let i = eval(w, fr, idx);
            let n = list(w, fr, *l).len() + 1;
            let ix = w.list_index(&i, n, false);
            list(w, fr, *l).insert(ix, x);
        }
        Stmt::ListReplace(l, idx, item) => {
            // Inputs in block order; the index is resolved (possibly drawing
            // a random number) when the block runs.
            let i = eval(w, fr, idx);
            let n = list(w, fr, *l).len();
            let x = eval(w, fr, item);
            let ix = w.list_index(&i, n, false);
            list(w, fr, *l).replace(ix, x);
        }
        Stmt::Cmd(cmd, args, order) => {
            let _ = command(w, fr, *cmd, args, order);
        }
        Stmt::Discard(args) => discard(w, fr, args),
        Stmt::StopOtherScripts => w.stop_other_scripts(me),
        Stmt::StopThisScript => return Flow::Stop,
        Stmt::Broadcast(b) => {
            broadcast(w, fr, b);
        }
        Stmt::If(c, a, b) => {
            let body = if boolean(w, fr, c) { a } else { b };
            return block_sync(w, fr, body);
        }
        Stmt::Repeat(times, body) => {
            let mut n = js_round(num(w, fr, times));
            loop {
                n -= 1.0;
                if !(n >= 0.0) {
                    break;
                }
                if block_sync(w, fr, body) == Flow::Stop {
                    return Flow::Stop;
                }
            }
        }
        Stmt::Forever(body) => loop {
            if block_sync(w, fr, body) == Flow::Stop {
                return Flow::Stop;
            }
        },
        Stmt::RepeatUntil(c, body) | Stmt::While(c, body) => {
            let until = matches!(s, Stmt::RepeatUntil(..));
            loop {
                if boolean(w, fr, c) == until {
                    break;
                }
                if block_sync(w, fr, body) == Flow::Stop {
                    return Flow::Stop;
                }
            }
        }
        Stmt::ForEach(v, value, body) => {
            let mut fe = 0.0_f64;
            loop {
                let lim = eval(w, fr, value).js_number();
                if !(fe < lim) {
                    break;
                }
                fe += 1.0;
                *var(w, fr, *v, false) = Value::Num(fe);
                if block_sync(w, fr, body) == Flow::Stop {
                    return Flow::Stop;
                }
            }
        }
        Stmt::Call { proc, args, order } => {
            let cw = interp().an.callee_warp(&interp().program, *proc, fr.warp);
            let vals = call_args(w, fr, args, order);
            call_sync(w, fr, *proc, cw, vals);
        }
        // Statements that may suspend never reach here in a function that
        // cannot suspend (the analysis makes such functions asynchronous).
        // Their side effects without the waiting:
        Stmt::StopAll => w.stop_all(),
        Stmt::DeleteThisClone => {
            w.delete_this_clone(me);
        }
        Stmt::BroadcastAndWait(b) => {
            broadcast(w, fr, b);
        }
        Stmt::Wait(_)
        | Stmt::WaitUntil(_)
        | Stmt::MusicWait { .. }
        | Stmt::Glide { .. }
        | Stmt::GlideTo { .. }
        | Stmt::SayForSecs { .. }
        | Stmt::Ask(_)
        | Stmt::PlaySoundUntilDone(_)
        | Stmt::SwitchBackdropAndWait(_) => {}
    }
    Flow::Next
}

fn call_sync(w: &mut World, fr: &Frame, proc: ProcId, warp: bool, args: Vec<Value>) {
    let body = &interp().program.targets[proc.target].procs[proc.index].body;
    let callee = proc_frame(fr, proc, warp, false, args);
    let _ = block_sync(w, &callee, body);
}

// ------------------------------------------------------------------ asynchronous statements

/// A statement list of an asynchronous function.
fn block<'a>(fr: &'a Frame, body: &'a [Stmt], tail: Tail) -> Fut<'a> {
    Box::pin(async move {
        let n = body.len();
        for (i, s) in body.iter().enumerate() {
            let t = if i + 1 == n { tail } else { Tail::No };
            let flow = if fr.is_async && interp().may_yield(s, fr.warp) {
                stmt_async(fr, s, t).await
            } else {
                let mut g = world();
                stmt_sync(&mut g, fr, s)
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Next
    })
}

/// Evaluate with the world borrowed; the borrow ends before any `.await`.
fn with<R>(f: impl FnOnce(&mut World) -> R) -> R {
    let mut g = world();
    f(&mut g)
}

/// The yield at the end of a loop iteration.
async fn loop_end(fr: &Frame, body: &[Stmt]) {
    if !fr.warp {
        yield_loop().await;
    } else {
        if interp().has_promise(body) {
            world().set_skip_loop_yield(false);
        }
        if interp().an.opts.warp_timer && world().warp_stuck() {
            yield_now().await;
        }
    }
}

/// The (possibly busy-waiting) yield of a re-executing block.
async fn yield_retry(fr: &Frame) {
    if fr.warp {
        if world().warp_stuck() {
            yield_now().await;
        }
    } else {
        yield_now().await;
    }
}

fn promise_done(tail: Tail) {
    if tail != Tail::No {
        world().set_skip_loop_yield(true);
    }
}

/// The waiting half of `broadcast and wait` / `switch backdrop and wait`.
async fn wait_threads(fr: &Frame, th: &[u64]) {
    if th.is_empty() {
        return;
    }
    loop {
        let st = world().wait_threads_status(th);
        match st {
            None => break,
            Some(s) => {
                if fr.warp {
                    if s == Status::YieldTick {
                        yield_tick().await;
                    } else if world().warp_stuck() {
                        yield_now().await;
                    }
                } else {
                    yield_status(s).await;
                }
            }
        }
    }
}

/// Wait on a stack timer (`wait`, music blocks).
async fn wait_timer(fr: &Frame, tm: (f64, f64)) {
    if !fr.warp {
        yield_now().await;
    }
    loop {
        if world().wait_done(tm) {
            break;
        }
        yield_retry(fr).await;
    }
}

fn stmt_async<'a>(fr: &'a Frame, s: &'a Stmt, tail: Tail) -> Fut<'a> {
    Box::pin(async move {
        let me = fr.me;
        match s {
            Stmt::If(c, a, b) => {
                let cv = with(|w| boolean(w, fr, c));
                return block(fr, if cv { a } else { b }, tail).await;
            }
            Stmt::Repeat(times, body) => {
                let mut n = with(|w| js_round(num(w, fr, times)));
                loop {
                    n -= 1.0;
                    if !(n >= 0.0) {
                        break;
                    }
                    if block(fr, body, Tail::Loop).await == Flow::Stop {
                        return Flow::Stop;
                    }
                    loop_end(fr, body).await;
                }
            }
            Stmt::Forever(body) => loop {
                if block(fr, body, Tail::Loop).await == Flow::Stop {
                    return Flow::Stop;
                }
                loop_end(fr, body).await;
            },
            Stmt::RepeatUntil(c, body) | Stmt::While(c, body) => {
                let until = matches!(s, Stmt::RepeatUntil(..));
                loop {
                    let cv = with(|w| boolean(w, fr, c));
                    if cv == until {
                        break;
                    }
                    if block(fr, body, Tail::Loop).await == Flow::Stop {
                        return Flow::Stop;
                    }
                    loop_end(fr, body).await;
                }
            }
            Stmt::ForEach(v, value, body) => {
                let mut fe = 0.0_f64;
                loop {
                    let go = with(|w| {
                        let lim = eval(w, fr, value).js_number();
                        if !(fe < lim) {
                            return false;
                        }
                        fe += 1.0;
                        *var(w, fr, *v, false) = Value::Num(fe);
                        true
                    });
                    if !go {
                        break;
                    }
                    if block(fr, body, Tail::Loop).await == Flow::Stop {
                        return Flow::Stop;
                    }
                    loop_end(fr, body).await;
                }
            }
            Stmt::Wait(secs) => {
                let tm = with(|w| {
                    let s = num(w, fr, secs);
                    w.wait_begin(s)
                });
                wait_timer(fr, tm).await;
            }
            Stmt::MusicWait { kind, arg, beats } => {
                // Inputs once, then a stack timer re-checked on every re-execution.
                let tm = with(|w| {
                    let _ = eval(w, fr, arg);
                    let b = num(w, fr, beats);
                    w.music_begin(*kind, b)
                });
                if let Some(tm) = tm {
                    wait_timer(fr, tm).await;
                }
            }
            Stmt::WaitUntil(c) => loop {
                if with(|w| boolean(w, fr, c)) {
                    break;
                }
                yield_retry(fr).await;
            },
            Stmt::StopAll => {
                world().stop_all();
                die().await;
            }
            Stmt::DeleteThisClone => {
                if world().delete_this_clone(me) {
                    die().await;
                }
            }
            Stmt::BroadcastAndWait(b) => {
                let th = with(|w| broadcast(w, fr, b));
                wait_threads(fr, &th).await;
            }
            Stmt::SwitchBackdropAndWait(e) => {
                let th = with(|w| {
                    let v = eval(w, fr, e);
                    w.switch_backdrop_threads(v)
                });
                // Bug-compatible with scratch-vm: starting hats here aborts the tick.
                if !th.is_empty() {
                    world().abort_tick();
                    yield_now().await;
                }
                wait_threads(fr, &th).await;
            }
            Stmt::Glide { secs, x, y } => {
                let g = with(|w| {
                    let (s, x, y) = (num(w, fr, secs), num(w, fr, x), num(w, fr, y));
                    w.glide_begin(me, s, x, y)
                });
                if let Some(g) = g {
                    loop {
                        yield_retry(fr).await;
                        if world().glide_step(me, &g) {
                            break;
                        }
                    }
                }
            }
            Stmt::GlideTo { secs, to } => {
                // `getTargetXY` runs again on every re-execution (it may draw random numbers).
                let dynamic_to = !matches!(to, Expr::Lit(Value::Str(s)) if !s.eq_str("_random_"));
                let g = with(|w| {
                    let s = num(w, fr, secs);
                    let t = eval(w, fr, to);
                    match w.target_xy(&t) {
                        Some((x, y)) => w.glide_begin(me, s, x, y),
                        None => None,
                    }
                });
                if let Some(g) = g {
                    loop {
                        yield_retry(fr).await;
                        let done = with(|w| {
                            if dynamic_to {
                                let t = eval(w, fr, to);
                                match w.target_xy(&t) {
                                    Some(_) => w.glide_step(me, &g),
                                    None => true,
                                }
                            } else {
                                w.glide_step(me, &g)
                            }
                        });
                        if done {
                            break;
                        }
                    }
                }
            }
            Stmt::SayForSecs { think, message, secs } => {
                let wk = with(|w| {
                    let m = eval(w, fr, message);
                    let s = eval(w, fr, secs);
                    w.say_for_secs(me, *think, m, s)
                });
                wait_external(wk).await;
                promise_done(tail);
            }
            Stmt::Ask(q) => {
                let wk = with(|w| {
                    let q = string(w, fr, q);
                    w.ask(me, q)
                });
                wait_external(wk).await;
                promise_done(tail);
            }
            Stmt::PlaySoundUntilDone(e) => {
                let wk = with(|w| {
                    let s = eval(w, fr, e);
                    w.sound_play_until_done(me, s)
                });
                if let Some(wk) = wk {
                    wait_external(wk).await;
                    promise_done(tail);
                }
            }
            Stmt::Cmd(cmd, args, order) => {
                if let Some(wk) = with(|w| command(w, fr, *cmd, args, order)) {
                    wait_external(wk).await;
                    promise_done(tail);
                }
            }
            Stmt::Call { proc, args, order } => {
                let it = interp();
                let cw = it.an.callee_warp(&it.program, *proc, fr.warp);
                // TurboWarp yields before a procedure calls itself outside
                // warp mode (`yieldForRecursion`), ahead of the arguments.
                if !fr.warp && fr.is_async && fr.proc == Some(*proc) {
                    yield_now().await;
                }
                let vals = with(|w| call_args(w, fr, args, order));
                if !proc_is_async(*proc, cw) {
                    with(|w| call_sync(w, fr, *proc, cw, vals));
                } else {
                    let body = &it.program.targets[proc.target].procs[proc.index].body;
                    let callee = proc_frame(fr, *proc, cw, true, vals);
                    let _ = block(&callee, body, Tail::Fn).await;
                    if tail == Tail::No && it.an.promise_tail.contains(proc) {
                        world().set_skip_loop_yield(false);
                    }
                }
            }
            // Everything else cannot suspend.
            _ => {
                let mut g = world();
                return stmt_sync(&mut g, fr, s);
            }
        }
        Flow::Next
    })
}
