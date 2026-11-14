//! scrust IR: a structured, name-resolved form of a Scratch project.
//!
//! * variables / lists / procedures / broadcasts are resolved at compile time
//!   exactly the way scratch-vm resolves them at run time;
//! * literals keep their original type (string vs number);
//! * control flow is structural (no block ids).

pub mod lower;
pub mod tables;

pub use lower::{lower, LowerOptions};
pub use scrust_value::{MathOp, Value};
pub use tables::{Cmd, Rep, A, K, R};

/// A scalar variable: `target` indexes `Program::targets` (0 = stage).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId {
    pub target: usize,
    pub index: usize,
}

/// A list variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ListId {
    pub target: usize,
    pub index: usize,
}

/// A procedure: `target` indexes `Program::targets`, `index` its `procs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcId {
    pub target: usize,
    pub index: usize,
}

#[derive(Clone, Debug)]
pub struct Program {
    /// `[0]` is the stage; sprites follow in layer order (back to front).
    pub targets: Vec<TargetIr>,
    /// Distinct broadcast names, upper-cased (the key hats are matched on).
    pub messages: Vec<String>,
    /// Stage broadcast variables in lookup order: (lower-cased name, message index).
    pub broadcast_lookup: Vec<(String, usize)>,
    /// Unsupported opcodes encountered: (target name, opcode).
    pub unsupported: Vec<(String, String)>,
    pub extensions: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TargetIr {
    pub name: String,
    pub is_stage: bool,
    pub vars: Vec<VarDecl>,
    pub lists: Vec<ListDecl>,
    pub scripts: Vec<Script>,
    pub procs: Vec<Proc>,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub id: String,
    pub name: String,
    pub init: Value,
    pub cloud: bool,
}

#[derive(Clone, Debug)]
pub struct ListDecl {
    pub id: String,
    pub name: String,
    pub init: Vec<Value>,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub hat: Hat,
    pub body: Vec<Stmt>,
    pub block_id: String,
}

#[derive(Clone, Debug)]
pub enum Hat {
    FlagClicked,
    KeyPressed(String),
    SpriteClicked,
    StageClicked,
    /// Upper-cased backdrop name.
    BackdropSwitchesTo(String),
    /// Edge-activated: `when [timer|loudness] > (value)`.
    GreaterThan { menu: String, value: Expr },
    /// Message index into `Program::messages`.
    BroadcastReceived(usize),
    StartAsClone,
    /// Edge-activated: `when touching (object)`.
    TouchingObject(Expr),
}

impl Hat {
    pub fn is_edge_activated(&self) -> bool {
        matches!(self, Hat::GreaterThan { .. } | Hat::TouchingObject(_))
    }
    /// scratch-vm `restartExistingThreads`.
    pub fn restarts_existing(&self) -> bool {
        matches!(
            self,
            Hat::FlagClicked | Hat::SpriteClicked | Hat::StageClicked | Hat::BackdropSwitchesTo(_) | Hat::BroadcastReceived(_)
        )
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub id: String,
    /// Value used when the call site leaves the slot empty.
    pub default: Value,
}

#[derive(Clone, Debug)]
pub struct Proc {
    pub proccode: String,
    pub params: Vec<Param>,
    pub warp: bool,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub enum BroadcastRef {
    /// Statically known message.
    Static(usize),
    /// The referenced broadcast does not exist: nothing is started.
    Nothing,
    /// A reporter is plugged in; resolved by name at run time.
    Dynamic(Expr),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    SetVar(VarId, Expr),
    ChangeVar(VarId, Expr),
    ShowVar(VarId, bool),
    ShowList(ListId, bool),
    ListAdd(ListId, Expr),
    ListDelete(ListId, Expr),
    ListDeleteAll(ListId),
    /// (list, index, item)
    ListInsert(ListId, Expr, Expr),
    /// (list, index, item)
    ListReplace(ListId, Expr, Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    Repeat(Expr, Vec<Stmt>),
    Forever(Vec<Stmt>),
    RepeatUntil(Expr, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    ForEach(VarId, Expr, Vec<Stmt>),
    Wait(Expr),
    WaitUntil(Expr),
    StopAll,
    StopThisScript,
    StopOtherScripts,
    DeleteThisClone,
    /// Arguments are in parameter order; `order` is the evaluation order.
    Call { proc: ProcId, args: Vec<Expr>, order: Vec<usize> },
    Broadcast(BroadcastRef),
    BroadcastAndWait(BroadcastRef),
    /// Table-driven command. `order` is the evaluation order of `args`.
    Cmd(Cmd, Vec<Expr>, Vec<usize>),
    /// Inputs that are evaluated for their side effects only.
    Discard(Vec<Expr>),
    Glide { secs: Expr, x: Expr, y: Expr },
    GlideTo { secs: Expr, to: Expr },
    SayForSecs { think: bool, message: Expr, secs: Expr },
    Ask(Expr),
    PlaySoundUntilDone(Expr),
    SwitchBackdropAndWait(Expr),
    /// Music extension: play a note (`kind` 0) or drum (1) for some beats, or
    /// rest (2). `arg` is the note / drum input (evaluated, not played).
    MusicWait { kind: u8, arg: Expr, beats: Expr },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Round,
    Lt,
    Eq,
    Gt,
    And,
    Or,
    Not,
    Join,
    LetterOf,
    Length,
    Contains,
    Random,
    Math(MathOp),
    /// `operator_mathop` with a non-literal operator name: (name, num).
    MathDyn,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Value),
    /// An empty slot (`undefined` in scratch-vm).
    Missing,
    Var(VarId),
    /// Read a variable of the original (non-clone) instance of another sprite.
    VarOf(VarId),
    ListContents(ListId),
    /// Procedure parameter by index.
    Arg(usize),
    /// Operator; `order` (when present) gives the evaluation order of the args.
    Op(Op, Vec<Expr>, Option<Vec<usize>>),
    /// Table-driven reporter.
    Rep(Rep, Vec<Expr>),
    ListItem(ListId, Box<Expr>),
    ListIndexOf(ListId, Box<Expr>),
    ListLen(ListId),
    ListContains(ListId, Box<Expr>),
}

impl Expr {
    pub fn num(n: f64) -> Expr {
        Expr::Lit(Value::Num(n))
    }
    pub fn str(s: &str) -> Expr {
        Expr::Lit(Value::str(s))
    }

    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Op(_, a, _) | Expr::Rep(_, a) => a.iter().collect(),
            Expr::ListItem(_, e) | Expr::ListIndexOf(_, e) | Expr::ListContains(_, e) => vec![e],
            _ => vec![],
        }
    }

    /// May evaluating this expression consume randomness (the only
    /// observable effect a reporter can have)? Conservative.
    pub fn is_impure(&self) -> bool {
        match self {
            Expr::Op(Op::Random, _, _) => true,
            Expr::ListItem(_, idx) => {
                // "random" / "any" indices draw from the RNG.
                match &**idx {
                    Expr::Lit(Value::Num(_)) => false,
                    Expr::Lit(Value::Str(s)) => s.eq_str("random") || s.eq_str("any"),
                    Expr::Lit(Value::Bool(_)) => false,
                    Expr::Op(op, args, _) if !matches!(op, Op::Join | Op::LetterOf) => args.iter().any(|a| a.is_impure()),
                    _ => true,
                }
            }
            _ => self.children().iter().any(|c| c.is_impure()),
        }
    }
}

impl Stmt {
    /// Nested statement lists.
    pub fn bodies(&self) -> Vec<&Vec<Stmt>> {
        match self {
            Stmt::If(_, a, b) => vec![a, b],
            Stmt::Repeat(_, b) | Stmt::Forever(b) | Stmt::RepeatUntil(_, b) | Stmt::While(_, b) | Stmt::ForEach(_, _, b) => vec![b],
            _ => vec![],
        }
    }

    /// Expressions evaluated directly by this statement.
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Stmt::SetVar(_, e)
            | Stmt::ChangeVar(_, e)
            | Stmt::ListAdd(_, e)
            | Stmt::ListDelete(_, e)
            | Stmt::If(e, _, _)
            | Stmt::Repeat(e, _)
            | Stmt::RepeatUntil(e, _)
            | Stmt::While(e, _)
            | Stmt::ForEach(_, e, _)
            | Stmt::Wait(e)
            | Stmt::WaitUntil(e)
            | Stmt::Ask(e)
            | Stmt::PlaySoundUntilDone(e)
            | Stmt::SwitchBackdropAndWait(e) => vec![e],
            Stmt::ListInsert(_, a, b) | Stmt::ListReplace(_, a, b) => vec![a, b],
            Stmt::Call { args, .. } | Stmt::Cmd(_, args, _) | Stmt::Discard(args) => args.iter().collect(),
            Stmt::Broadcast(BroadcastRef::Dynamic(e)) | Stmt::BroadcastAndWait(BroadcastRef::Dynamic(e)) => vec![e],
            Stmt::Glide { secs, x, y } => vec![secs, x, y],
            Stmt::GlideTo { secs, to } => vec![secs, to],
            Stmt::SayForSecs { message, secs, .. } => vec![message, secs],
            Stmt::MusicWait { arg, beats, .. } => vec![arg, beats],
            _ => vec![],
        }
    }
}

/// Walk every statement (pre-order) in a body.
pub fn walk_stmts<'a>(body: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt)) {
    for s in body {
        f(s);
        for b in s.bodies() {
            walk_stmts(b, f);
        }
    }
}

/// Walk every expression node (pre-order) reachable from a body.
pub fn walk_exprs<'a>(body: &'a [Stmt], f: &mut dyn FnMut(&'a Expr)) {
    fn rec<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
        f(e);
        for c in e.children() {
            rec(c, f);
        }
    }
    walk_stmts(body, &mut |s| {
        for e in s.exprs() {
            rec(e, f);
        }
    });
}
