//! Whole-program analyses: storage types (§8.4), kind-sensitivity (§8.6),
//! storage representation (§8.7), yield analysis and procedure variants (§10.5).

pub mod ty;

use scrust_ir::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
pub use ty::Ty;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Keep every variable, list and parameter as a dynamic `Value`.
    pub no_unbox: bool,
    /// Loops in warp mode may yield (500 ms warp timer).
    pub warp_timer: bool,
    /// Variables / lists with an on-stage monitor.
    pub monitored_vars: Vec<VarId>,
    pub monitored_lists: Vec<ListId>,
    /// Some costume, backdrop or sound is named like a canonical number.
    pub numeric_asset_names: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Repr {
    F64,
    Bool,
    Str,
    Val,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    Var(VarId),
    List(ListId),
    Param(ProcId, usize),
}

/// Where a body lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Script(usize, usize),
    Proc(ProcId),
}

impl Owner {
    pub fn proc(self) -> Option<ProcId> {
        match self {
            Owner::Proc(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Analysis {
    pub opts: Options,
    pub types: HashMap<Node, Ty>,
    pub reprs: HashMap<Node, Repr>,
    /// Nodes whose string/number kind is observable for any canonical string.
    pub kind_sensitive: HashSet<Node>,
    /// Nodes whose kind is observable only for fractional canonical strings.
    pub fract_sensitive: HashSet<Node>,
    /// May the (procedure, warp) variant suspend the thread?
    pub yields: HashMap<(ProcId, bool), bool>,
    /// Variants that are actually reachable.
    pub variants: BTreeSet<(ProcId, bool)>,
    /// Procedures on a call cycle (recursion checks apply).
    pub cyclic: HashSet<ProcId>,
    /// Procedures that may finish on a promise block (loop-yield skipping).
    pub promise_tail: HashSet<ProcId>,
}

fn for_each_body<'a>(p: &'a Program, f: &mut dyn FnMut(Owner, &'a [Stmt])) {
    for (ti, t) in p.targets.iter().enumerate() {
        for (si, s) in t.scripts.iter().enumerate() {
            f(Owner::Script(ti, si), &s.body);
        }
        for (pi, pr) in t.procs.iter().enumerate() {
            f(Owner::Proc(ProcId { target: ti, index: pi }), &pr.body);
        }
    }
}

impl Analysis {
    pub fn run(p: &Program, opts: Options) -> Analysis {
        let mut a = Analysis { opts, ..Default::default() };
        a.storage_types(p);
        a.sensitivity(p);
        a.choose_reprs(p);
        a.call_graph(p);
        a
    }

    fn ty(&self, n: Node) -> Ty {
        self.types.get(&n).copied().unwrap_or(Ty::NONE)
    }

    pub fn repr(&self, n: Node) -> Repr {
        self.reprs.get(&n).copied().unwrap_or(Repr::Val)
    }

    /// Type observed when reading a storage node.
    pub fn read_ty(&self, n: Node) -> Ty {
        let t = self.ty(n);
        match self.reprs.get(&n) {
            Some(Repr::F64) => t.numeric_image(),
            _ => t,
        }
    }

    /// Static type of an expression evaluated inside `owner`.
    pub fn ty_of(&self, e: &Expr, owner: Owner) -> Ty {
        match e {
            Expr::Lit(v) => Ty::of_value(v),
            Expr::Missing => Ty::STR_NAN,
            Expr::Var(v) | Expr::VarOf(v) => self.read_ty(Node::Var(*v)),
            Expr::ListContents(_) => Ty::STRING,
            Expr::Arg(i) => match owner.proc() {
                Some(p) => {
                    let t = self.read_ty(Node::Param(p, *i));
                    if t.is_empty() {
                        Ty::ANY
                    } else {
                        t
                    }
                }
                None => Ty::ZERO,
            },
            Expr::Op(op, args, _) => {
                if let Some(v) = fold(e) {
                    return Ty::of_value(&v);
                }
                // Operand types; constants contribute their exact numeric class
                // (`"2"` is a positive integer, not just "some integer").
                let t = |i: usize| match fold(&args[i]) {
                    Some(v) if matches!(op, Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Round | Op::Random | Op::Math(_)) => {
                        Ty::of_number(scrust_value::ToNum::to_num(&v))
                    }
                    _ => self.ty_of(&args[i], owner),
                };
                match op {
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod => ty::arith(*op, t(0), t(1)),
                    Op::Round => {
                        let n = t(0).to_num();
                        Ty(n.0 & !Ty::FRACT.0) | if n.has(Ty::FRACT) { Ty::INT | Ty::NEG_ZERO } else { Ty::NONE }
                    }
                    Op::Lt | Op::Eq | Op::Gt | Op::And | Op::Or | Op::Not | Op::Contains => Ty::BOOLEAN,
                    Op::Join => Ty::STRING,
                    Op::LetterOf => Ty::STR_CINT | Ty::STR_NAN,
                    Op::Length => Ty::POS_INT | Ty::ZERO,
                    Op::Random => {
                        let both = t(0).to_num() | t(1).to_num();
                        if both.has(Ty::INF) {
                            Ty::NUMBER_OR_NAN
                        } else {
                            Ty::NUMBER
                        }
                    }
                    Op::Math(m) => {
                        let n = t(0).to_num();
                        match m {
                            MathOp::Abs => Ty(n.0 & !(Ty::NEG_INT.0 | Ty::NEG_FRACT.0 | Ty::NEG_INF.0 | Ty::NEG_ZERO.0)) | {
                                let mut r = Ty::NONE;
                                if n.has(Ty::NEG_INT) {
                                    r |= Ty::POS_INT;
                                }
                                if n.has(Ty::NEG_FRACT) {
                                    r |= Ty::POS_FRACT;
                                }
                                if n.has(Ty::NEG_INF) {
                                    r |= Ty::POS_INF;
                                }
                                if n.has(Ty::NEG_ZERO) {
                                    r |= Ty::ZERO;
                                }
                                r
                            },
                            MathOp::Floor | MathOp::Ceiling => {
                                Ty(n.0 & !Ty::FRACT.0) | if n.has(Ty::FRACT) { Ty::INT | Ty::NEG_ZERO } else { Ty::NONE }
                            }
                            MathOp::Unknown => Ty::ZERO,
                            _ => Ty::NUMBER_OR_NAN,
                        }
                    }
                    Op::MathDyn => Ty::NUMBER_OR_NAN,
                }
            }
            Expr::Rep(rep, _) => match rep.result() {
                R::Num => Ty::NUMBER,
                R::Bool => Ty::BOOLEAN,
                R::Str => Ty::STRING,
                R::Val => Ty::ANY,
            },
            Expr::ListItem(l, _) => self.read_ty(Node::List(*l)) | Ty::STR_NAN,
            Expr::ListIndexOf(..) | Expr::ListLen(_) => Ty::POS_INT | Ty::ZERO,
            Expr::ListContains(..) => Ty::BOOLEAN,
        }
    }

    // ---------------------------------------------------------------- layer 1

    fn storage_types(&mut self, p: &Program) {
        for (ti, t) in p.targets.iter().enumerate() {
            for (i, v) in t.vars.iter().enumerate() {
                self.types.insert(Node::Var(VarId { target: ti, index: i }), Ty::of_value(&v.init));
            }
            for (i, l) in t.lists.iter().enumerate() {
                let ty = l.init.iter().fold(Ty::NONE, |a, v| a | Ty::of_value(v));
                self.types.insert(Node::List(ListId { target: ti, index: i }), ty);
            }
        }
        loop {
            let mut changed = false;
            let mut writes: Vec<(Node, Ty)> = Vec::new();
            for_each_body(p, &mut |owner, body| {
                walk_stmts(body, &mut |s| match s {
                    Stmt::SetVar(v, e) => writes.push((Node::Var(*v), self.ty_of(e, owner))),
                    Stmt::ChangeVar(v, e) => {
                        writes.push((Node::Var(*v), ty::arith(Op::Add, self.ty(Node::Var(*v)), self.ty_of(e, owner))))
                    }
                    Stmt::ForEach(v, _, _) => writes.push((Node::Var(*v), Ty::POS_INT)),
                    Stmt::ListAdd(l, e) | Stmt::ListInsert(l, _, e) | Stmt::ListReplace(l, _, e) => {
                        writes.push((Node::List(*l), self.ty_of(e, owner)))
                    }
                    Stmt::Call { proc, args, .. } => {
                        for (i, a) in args.iter().enumerate() {
                            writes.push((Node::Param(*proc, i), self.ty_of(a, owner)));
                        }
                    }
                    _ => {}
                });
            });
            for (n, t) in writes {
                let cur = self.ty(n);
                if !t.within(cur) {
                    self.types.insert(n, cur | t);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    // ------------------------------------------------------------ sensitivity

    fn raw_sources(e: &Expr, owner: Owner) -> Vec<Node> {
        match e {
            Expr::Var(v) | Expr::VarOf(v) => vec![Node::Var(*v)],
            Expr::Arg(i) => owner.proc().map(|p| vec![Node::Param(p, *i)]).unwrap_or_default(),
            Expr::ListItem(l, _) => vec![Node::List(*l)],
            _ => vec![],
        }
    }

    fn sensitivity(&mut self, p: &Program) {
        let mut edges: Vec<(Node, Node)> = Vec::new();
        let mut kind: HashSet<Node> = HashSet::new();
        let mut fract: HashSet<Node> = HashSet::new();
        let numeric_names = self.opts.numeric_asset_names;
        for v in &self.opts.monitored_vars {
            fract.insert(Node::Var(*v));
        }
        for l in &self.opts.monitored_lists {
            fract.insert(Node::List(*l));
        }
        for_each_body(p, &mut |owner, body| {
            let src = |e: &Expr| Self::raw_sources(e, owner);
            walk_stmts(body, &mut |s| match s {
                Stmt::SetVar(v, e) => edges.extend(src(e).into_iter().map(|n| (n, Node::Var(*v)))),
                Stmt::ListAdd(l, e) | Stmt::ListInsert(l, _, e) | Stmt::ListReplace(l, _, e) => {
                    edges.extend(src(e).into_iter().map(|n| (n, Node::List(*l))))
                }
                Stmt::Call { proc, args, .. } => {
                    for (i, a) in args.iter().enumerate() {
                        edges.extend(src(a).into_iter().map(|n| (n, Node::Param(*proc, i))));
                    }
                }
                Stmt::Cmd(cmd, args, _) => {
                    for (spec, e) in cmd.args().iter().zip(args) {
                        if let A::In(_, K::RawKind) = spec {
                            match cmd {
                                Cmd::Say | Cmd::Think => fract.extend(src(e)),
                                _ => {
                                    if numeric_names {
                                        kind.extend(src(e));
                                    }
                                }
                            }
                        }
                    }
                }
                Stmt::SayForSecs { message, .. } => fract.extend(src(message)),
                Stmt::SwitchBackdropAndWait(e) | Stmt::PlaySoundUntilDone(e) => {
                    if numeric_names {
                        kind.extend(src(e));
                    }
                }
                Stmt::Discard(args) => {
                    for a in args {
                        kind.extend(src(a));
                    }
                }
                _ => {}
            });
            walk_exprs(body, &mut |e| match e {
                Expr::ListContents(l) => {
                    kind.insert(Node::List(*l));
                }
                Expr::Rep(Rep::KeyPressed, args) => kind.extend(args.iter().flat_map(|a| Self::raw_sources(a, owner))),
                Expr::Rep(Rep::AttrOf, args) => {
                    // A dynamic `(property) of (object)`: any variable with that name may be read raw.
                    if let Some(Expr::Lit(prop)) = args.first() {
                        let name = scrust_value::ToStr::to_str(prop).to_rust_string();
                        for (ti, t) in p.targets.iter().enumerate() {
                            for (i, v) in t.vars.iter().enumerate() {
                                if v.name == name {
                                    kind.insert(Node::Var(VarId { target: ti, index: i }));
                                }
                            }
                        }
                    }
                }
                _ => {}
            });
        });
        // Hat inputs.
        for (ti, t) in p.targets.iter().enumerate() {
            for (si, s) in t.scripts.iter().enumerate() {
                if let Hat::TouchingObject(e) = &s.hat {
                    let _ = (e, ti, si);
                }
            }
        }
        // Backward closure over "raw value flows into".
        loop {
            let mut changed = false;
            for (from, to) in &edges {
                if kind.contains(to) && kind.insert(*from) {
                    changed = true;
                }
                if fract.contains(to) && fract.insert(*from) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.kind_sensitive = kind;
        self.fract_sensitive = fract;
    }

    fn choose_reprs(&mut self, _p: &Program) {
        let nodes: Vec<Node> = self.types.keys().copied().collect();
        for n in nodes {
            let t = self.ty(n);
            let repr = if self.opts.no_unbox || t.is_empty() {
                Repr::Val
            } else if t.within(Ty::NUMBER_OR_NAN) {
                Repr::F64
            } else if t.within(Ty::BOOLEAN) {
                Repr::Bool
            } else if t.within(Ty::NUMBER_OR_NAN | Ty::STR_CANON) && self.can_numify(n, t) {
                Repr::F64
            } else if t.within(Ty::STRING) {
                Repr::Str
            } else {
                Repr::Val
            };
            self.reprs.insert(n, repr);
        }
    }

    /// May canonical numeric strings in this node be stored as numbers (§8.6)?
    fn can_numify(&self, n: Node, t: Ty) -> bool {
        if self.kind_sensitive.contains(&n) {
            return false;
        }
        !(t.has(Ty::STR_CFRACT) && self.fract_sensitive.contains(&n))
    }

    /// Human-readable explanation of a node's representation.
    pub fn explain(&self, n: Node) -> String {
        let t = self.ty(n);
        let mut notes = Vec::new();
        if self.kind_sensitive.contains(&n) {
            notes.push("reaches a kind-sensitive sink");
        }
        if self.fract_sensitive.contains(&n) {
            notes.push("shown by say/think or a monitor");
        }
        if self.repr(n) == Repr::Val && !self.opts.no_unbox {
            if t.is_empty() {
                notes.push("never written");
            } else if t.has(Ty::STRING) && t.has(Ty::NUMBER_OR_NAN) {
                notes.push("holds both strings and numbers");
            } else if t.has(Ty::BOOLEAN) && !t.within(Ty::BOOLEAN) {
                notes.push("holds booleans and other values");
            }
        }
        format!("{:?} : {:?}{}", self.repr(n), t, if notes.is_empty() { String::new() } else { format!("  ({})", notes.join("; ")) })
    }

    // ------------------------------------------------------------- call graph

    fn call_graph(&mut self, p: &Program) {
        // Direct callees.
        let mut callees: BTreeMap<ProcId, BTreeSet<ProcId>> = BTreeMap::new();
        let mut all_procs: Vec<ProcId> = Vec::new();
        for (ti, t) in p.targets.iter().enumerate() {
            for pi in 0..t.procs.len() {
                let id = ProcId { target: ti, index: pi };
                all_procs.push(id);
                let set = callees.entry(id).or_default();
                walk_stmts(&t.procs[pi].body, &mut |s| {
                    if let Stmt::Call { proc, .. } = s {
                        set.insert(*proc);
                    }
                });
            }
        }
        // Cyclic procedures: p reaches p.
        for &start in &all_procs {
            let mut seen: BTreeSet<ProcId> = BTreeSet::new();
            let mut stack: Vec<ProcId> = callees[&start].iter().copied().collect();
            while let Some(q) = stack.pop() {
                if q == start {
                    self.cyclic.insert(start);
                    break;
                }
                if seen.insert(q) {
                    stack.extend(callees[&q].iter().copied());
                }
            }
        }
        // Promise tails (conservative: contains a promise block or calls one that does).
        loop {
            let mut changed = false;
            for &id in &all_procs {
                if self.promise_tail.contains(&id) {
                    continue;
                }
                let body = &p.targets[id.target].procs[id.index].body;
                let mut hit = false;
                walk_stmts(body, &mut |s| match s {
                    Stmt::SayForSecs { .. } | Stmt::Ask(_) | Stmt::PlaySoundUntilDone(_) => hit = true,
                    Stmt::Call { proc, .. } if self.promise_tail.contains(proc) => hit = true,
                    _ => {}
                });
                if hit {
                    self.promise_tail.insert(id);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Yield analysis: least fixpoint.
        for &id in &all_procs {
            self.yields.insert((id, false), false);
            self.yields.insert((id, true), false);
        }
        loop {
            let mut changed = false;
            for &id in &all_procs {
                let body = &p.targets[id.target].procs[id.index].body;
                for warp in [false, true] {
                    if !self.yields[&(id, warp)] && body.iter().any(|s| self.stmt_may_yield(p, s, warp)) {
                        self.yields.insert((id, warp), true);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // Reachable variants.
        let mut work: Vec<(ProcId, bool)> = Vec::new();
        for t in &p.targets {
            for s in &t.scripts {
                walk_stmts(&s.body, &mut |st| {
                    if let Stmt::Call { proc, .. } = st {
                        work.push((*proc, p.targets[proc.target].procs[proc.index].warp));
                    }
                });
            }
        }
        while let Some((id, warp)) = work.pop() {
            if !self.variants.insert((id, warp)) {
                continue;
            }
            walk_stmts(&p.targets[id.target].procs[id.index].body, &mut |st| {
                if let Stmt::Call { proc, .. } = st {
                    work.push((*proc, warp || p.targets[proc.target].procs[proc.index].warp));
                }
            });
        }
    }

    /// Does the callee run in warp mode when called from a `warp` context?
    pub fn callee_warp(&self, p: &Program, proc: ProcId, caller_warp: bool) -> bool {
        caller_warp || p.targets[proc.target].procs[proc.index].warp
    }

    /// May executing `s` suspend the thread (or terminate it from within)?
    pub fn stmt_may_yield(&self, p: &Program, s: &Stmt, warp: bool) -> bool {
        match s {
            Stmt::Wait(_)
            | Stmt::WaitUntil(_)
            | Stmt::BroadcastAndWait(_)
            | Stmt::Glide { .. }
            | Stmt::GlideTo { .. }
            | Stmt::SayForSecs { .. }
            | Stmt::Ask(_)
            | Stmt::PlaySoundUntilDone(_)
            | Stmt::SwitchBackdropAndWait(_)
            | Stmt::StopAll
            | Stmt::DeleteThisClone => true,
            Stmt::Repeat(_, b) | Stmt::Forever(b) | Stmt::RepeatUntil(_, b) | Stmt::While(_, b) | Stmt::ForEach(_, _, b) => {
                !warp || self.opts.warp_timer || b.iter().any(|x| self.stmt_may_yield(p, x, warp))
            }
            Stmt::If(_, a, b) => a.iter().chain(b).any(|x| self.stmt_may_yield(p, x, warp)),
            Stmt::Call { proc, .. } => {
                let cw = self.callee_warp(p, *proc, warp);
                self.yields.get(&(*proc, cw)).copied().unwrap_or(true) || (!cw && self.cyclic.contains(proc))
            }
            _ => false,
        }
    }
}

/// Constant folding with the exact runtime semantics.
pub fn fold(e: &Expr) -> Option<Value> {
    use scrust_value::{compare, ops, JsStr, ToBool, ToNum, ToStr};
    match e {
        Expr::Lit(v) => Some(v.clone()),
        Expr::Op(op, args, _) => {
            if matches!(op, Op::Random | Op::MathDyn) {
                return None;
            }
            let vals: Vec<Value> = args.iter().map(fold).collect::<Option<Vec<_>>>()?;
            let n = |i: usize| vals[i].to_num();
            let s = |i: usize| -> JsStr { vals[i].to_str() };
            let b = |i: usize| vals[i].to_bool();
            Some(match op {
                Op::Add => Value::Num(n(0) + n(1)),
                Op::Sub => Value::Num(n(0) - n(1)),
                Op::Mul => Value::Num(n(0) * n(1)),
                Op::Div => Value::Num(n(0) / n(1)),
                Op::Mod => Value::Num(ops::js_mod(n(0), n(1))),
                Op::Round => Value::Num(scrust_value::js_round(n(0))),
                Op::Lt => Value::Bool(compare(&vals[0], &vals[1]) < 0.0),
                Op::Eq => Value::Bool(compare(&vals[0], &vals[1]) == 0.0),
                Op::Gt => Value::Bool(compare(&vals[0], &vals[1]) > 0.0),
                Op::And => Value::Bool(b(0) && b(1)),
                Op::Or => Value::Bool(b(0) || b(1)),
                Op::Not => Value::Bool(!b(0)),
                Op::Join => Value::Str(ops::join(&s(0), &s(1))),
                Op::LetterOf => Value::Str(ops::letter_of(n(0), &s(1))),
                Op::Length => Value::Num(s(0).len() as f64),
                Op::Contains => Value::Bool(ops::contains(&s(0), &s(1))),
                Op::Math(m) => Value::Num(m.apply(n(0))),
                Op::Random | Op::MathDyn => return None,
            })
        }
        _ => None,
    }
}
