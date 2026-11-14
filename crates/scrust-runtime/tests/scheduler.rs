//! Scheduler tests on a hand-written program shaped exactly like generated
//! code (async scripts, cached storage pointers, clones). Also the target of
//! the Miri run described in docs/decisions/storage-pointers.md.

use scrust_runtime::prelude::*;
use scrust_runtime::run_headless;
use std::any::Any;

#[derive(Clone)]
struct L0 {
    counter: f64,
    log: List<Value>,
    text: Value,
}

#[derive(Clone)]
struct L1 {
    id: f64,
}

macro_rules! locals {
    ($t:ty, [$($name:literal => $f:ident),*]) => {
        impl Locals for $t {
            fn clone_box(&self) -> Box<dyn Locals> { Box::new(self.clone()) }
            fn as_any(&self) -> &dyn Any { self }
            fn as_any_mut(&mut self) -> &mut dyn Any { self }
            fn var_by_name(&self, name: &str) -> Option<Value> { match name { $($name => Some(self.$f.to_value()),)* _ => None } }
            fn vars(&self) -> Vec<(&'static str, Value)> { vec![$(($name, self.$f.to_value())),*] }
            fn lists(&self) -> Vec<(&'static str, Vec<Value>)> { vec![] }
            fn set_var(&mut self, _index: usize, _value: Value) {}
        }
    };
}
locals!(L0, ["counter" => counter, "text" => text]);
locals!(L1, ["id" => id]);

fn log(g: LocalsPtr<L0>, s: &str) {
    g.m().log.add(Value::str(s));
}

/// when flag clicked: forever { change counter by 1 }
fn counter_script(_me: TargetRef) -> BoxFut {
    Box::pin(async move {
        let g = world().globals_ptr::<L0>();
        loop {
            g.m().counter += 1.0;
            yield_loop().await;
        }
    })
}

/// when flag clicked: 3 clones, broadcast, wait, stop all
fn main_script(me: TargetRef) -> BoxFut {
    Box::pin(async move {
        let g = world().globals_ptr::<L0>();
        let l = world().locals_ptr::<L1>(me);
        for _ in 0..3 {
            l.m().id += 1.0;
            world().create_clone(me, JsStr::from("_myself_"));
            yield_loop().await;
        }
        log(g, "cloned");
        let started = world().broadcast(0);
        loop {
            // Bind first: a `world()` guard must not live across the `.await` below.
            let status = world().wait_threads_status(&started);
            match status {
                None => break,
                Some(s) => yield_status(s).await,
            }
        }
        log(g, "broadcast done");
        // A rope built across yields.
        for i in 0..50 {
            let cur = g.r().text.to_str();
            g.m().text = Value::Str(ops::join(&cur, &js_num_to_str(i as f64)));
            if i % 10 == 0 {
                yield_loop().await;
            }
        }
        let tm = world().wait_begin(0.1);
        yield_now().await;
        while !world().wait_done(tm) {
            yield_now().await;
        }
        log(g, "stopping");
        world().stop_all();
        die().await;
        log(g, "unreachable");
    })
}

/// when I start as a clone: log, then delete after a while
fn clone_script(me: TargetRef) -> BoxFut {
    Box::pin(async move {
        let g = world().globals_ptr::<L0>();
        let l = world().locals_ptr::<L1>(me);
        g.m().log.add(Value::Str(ops::join(&JsStr::from("clone "), &js_num_to_str(l.r().id))));
        for _ in 0..2 {
            yield_loop().await;
        }
        if l.r().id == 2.0 && world().delete_this_clone(me) {
            die().await;
        }
        loop {
            yield_loop().await;
        }
    })
}

/// when I receive [go]: restarts itself once
fn receive_script(me: TargetRef) -> BoxFut {
    Box::pin(async move {
        let g = world().globals_ptr::<L0>();
        let l = world().locals_ptr::<L1>(me);
        g.m().log.add(Value::Str(ops::join(&JsStr::from("recv "), &js_num_to_str(l.r().id))));
        yield_loop().await;
    })
}

fn script(kind: HatKind, restart: bool, start: fn(TargetRef) -> BoxFut) -> ScriptDef {
    ScriptDef { kind, field: "", msg: 0, restart, start, pred: None, creates: &[] }
}

fn sprite(name: &'static str, is_stage: bool, scripts: Vec<ScriptDef>, new_locals: fn() -> Box<dyn Locals>) -> SpriteDef {
    SpriteDef {
        name,
        is_stage,
        costumes: vec![],
        sounds: vec![],
        scripts,
        new_locals,
        x: 0.0,
        y: 0.0,
        size: 100.0,
        direction: 90.0,
        visible: true,
        draggable: false,
        costume: 0,
        rotation_style: RotationStyle::AllAround,
        volume: 100.0,
        declared_vars: usize::MAX,
        declared_lists: usize::MAX,
        tempo: 60.0,
    }
}

fn program() -> &'static ProgramDef {
    Box::leak(Box::new(ProgramDef {
        sprites: vec![
            sprite("Stage", true, vec![script(HatKind::Flag, true, counter_script)], || {
                Box::new(L0 { counter: 0.0, log: List::new(vec![]), text: Value::str("") })
            }),
            sprite(
                "Sprite",
                false,
                vec![script(HatKind::Flag, true, main_script), script(HatKind::Clone, false, clone_script), script(HatKind::Broadcast, true, receive_script)],
                || Box::new(L1 { id: 0.0 }),
            ),
        ],
        messages: vec!["GO"],
        broadcast_lookup: vec![("go", 0)],
        monitors: vec![],
    }))
}

#[test]
fn threads_clones_and_stop_all() {
    let options = Options { deterministic: true, max_passes: Some(2), ..Options::default() };
    let mut w = World::new(program(), options);
    let mut live = Vec::new();
    run_headless(&mut w, 12, |w| live.push((w.live_threads(), w.order.len())));
    let g = w.globals::<L0>();
    let entries: Vec<String> = g.log.items.iter().map(|v| v.to_str().to_rust_string()).collect();
    // Clones are born with the id at clone time; clone 2 has deleted itself before the broadcast.
    assert_eq!(&entries[..4], ["clone 1", "clone 2", "clone 3", "cloned"]);
    assert_eq!(entries.iter().filter(|e| e.starts_with("recv")).count(), 3);
    assert!(entries.contains(&"broadcast done".to_string()));
    assert_eq!(entries.last().map(String::as_str), Some("stopping"));
    assert!(g.text.to_str().len() > 50);
    // `stop all` removed the clones and every thread.
    assert_eq!(live.last(), Some(&(0, 2)));
    // Clone 2 deleted itself earlier: at some point there were exactly 2 clones.
    assert!(live.iter().any(|&(_, targets)| targets == 4));
    assert!(g.counter > 5.0);
}
