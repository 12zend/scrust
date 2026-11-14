//! sb3 AST -> IR lowering.

use crate::tables::{Cmd, Rep, A};
use crate::*;
use scrust_sb3::{Block, Input, Project, Target};
use scrust_value::{JsStr, ToStr};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct LowerOptions {}

struct Ctx<'a> {
    project: &'a Project,
    targets: Vec<TargetIr>,
    messages: Vec<String>,
    msg_index: HashMap<String, usize>,
    /// Broadcast variable id -> name, stage first.
    broadcast_by_id: HashMap<String, String>,
    broadcast_lookup: Vec<(String, usize)>,
    unsupported: Vec<(String, String)>,
}

struct ProcInfo {
    /// Parameter (name, id, default) from the first matching prototype.
    params: Vec<Param>,
    /// First matching definition block index, if any.
    def_block: Option<usize>,
    warp: bool,
}

/// Per-target lowering state.
struct TCtx<'a> {
    ti: usize,
    target: &'a Target,
    procs: Vec<ProcInfo>,
    proc_index: HashMap<String, usize>,
    /// Parameter names of the procedure whose body is being lowered.
    cur_params: Option<Vec<String>>,
}

fn js_upper(s: &str) -> String {
    JsStr::from(s).to_upper().to_rust_string()
}
fn js_lower(s: &str) -> String {
    JsStr::from(s).to_lower().to_rust_string()
}

pub fn lower(project: &Project, _opts: &LowerOptions) -> Program {
    let mut cx = Ctx {
        project,
        targets: Vec::new(),
        messages: Vec::new(),
        msg_index: HashMap::new(),
        broadcast_by_id: HashMap::new(),
        broadcast_lookup: Vec::new(),
        unsupported: Vec::new(),
    };
    for t in &project.targets {
        cx.targets.push(TargetIr {
            name: t.name.clone(),
            is_stage: t.is_stage,
            vars: t.variables.iter().map(|v| VarDecl { id: v.id.clone(), name: v.name.clone(), init: v.value.clone(), cloud: v.cloud }).collect(),
            lists: t.lists.iter().map(|l| ListDecl { id: l.id.clone(), name: l.name.clone(), init: l.items.clone() }).collect(),
            scripts: Vec::new(),
            procs: Vec::new(),
        });
    }
    // Broadcast variables: the stage is searched by scratch-vm; sprites'
    // declarations only serve as an id fallback.
    for (ti, t) in project.targets.iter().enumerate() {
        for (id, name) in &t.broadcasts {
            cx.broadcast_by_id.entry(id.clone()).or_insert_with(|| name.clone());
            if ti == 0 {
                let m = cx.message(name);
                cx.broadcast_lookup.push((js_lower(name), m));
            }
        }
    }
    for ti in 0..project.targets.len() {
        lower_target(&mut cx, ti);
    }
    Program {
        targets: cx.targets,
        messages: cx.messages,
        broadcast_lookup: cx.broadcast_lookup,
        unsupported: cx.unsupported,
        extensions: project.extensions.clone(),
    }
}

impl<'a> Ctx<'a> {
    fn message(&mut self, name: &str) -> usize {
        let key = js_upper(name);
        if let Some(&i) = self.msg_index.get(&key) {
            return i;
        }
        let i = self.messages.len();
        self.messages.push(key.clone());
        self.msg_index.insert(key, i);
        i
    }

    /// `lookupOrCreateVariable`: local id, stage id, local name, stage name, create local.
    fn resolve_var(&mut self, ti: usize, id: &str, name: &str) -> VarId {
        for t in [ti, 0] {
            if let Some(i) = self.targets[t].vars.iter().position(|v| v.id == id) {
                return VarId { target: t, index: i };
            }
        }
        for t in [ti, 0] {
            if let Some(i) = self.targets[t].vars.iter().position(|v| v.name == name) {
                return VarId { target: t, index: i };
            }
        }
        let vars = &mut self.targets[ti].vars;
        vars.push(VarDecl { id: id.to_string(), name: name.to_string(), init: Value::Num(0.0), cloud: false });
        VarId { target: ti, index: vars.len() - 1 }
    }

    fn resolve_list(&mut self, ti: usize, id: &str, name: &str) -> ListId {
        for t in [ti, 0] {
            if let Some(i) = self.targets[t].lists.iter().position(|v| v.id == id) {
                return ListId { target: t, index: i };
            }
        }
        for t in [ti, 0] {
            if let Some(i) = self.targets[t].lists.iter().position(|v| v.name == name) {
                return ListId { target: t, index: i };
            }
        }
        let lists = &mut self.targets[ti].lists;
        lists.push(ListDecl { id: id.to_string(), name: name.to_string(), init: Vec::new() });
        ListId { target: ti, index: lists.len() - 1 }
    }

    /// `lookupBroadcastMsg(id, name)` on the stage.
    fn resolve_broadcast(&mut self, id: Option<&str>, name: &str) -> BroadcastRef {
        if let Some(id) = id.filter(|i| !i.is_empty()) {
            return match self.broadcast_by_id.get(id).cloned() {
                Some(n) => BroadcastRef::Static(self.message(&n)),
                None => BroadcastRef::Nothing,
            };
        }
        if name.is_empty() {
            return BroadcastRef::Nothing;
        }
        let low = js_lower(name);
        match self.broadcast_lookup.iter().find(|(n, _)| *n == low) {
            Some(&(_, m)) => BroadcastRef::Static(m),
            None => BroadcastRef::Nothing,
        }
    }
}

fn lower_target(cx: &mut Ctx, ti: usize) {
    let project = cx.project;
    let target = &project.targets[ti];
    let mut tc = TCtx { ti, target, procs: Vec::new(), proc_index: HashMap::new(), cur_params: None };

    // Procedure tables: the first prototype with a proccode provides the
    // parameters, the first definition with that proccode provides the body.
    for b in &target.blocks {
        if b.opcode == "procedures_prototype" {
            if let Some(m) = &b.mutation {
                if !tc.proc_index.contains_key(&m.proccode) {
                    let params = m
                        .argument_ids
                        .iter()
                        .enumerate()
                        .map(|(i, id)| Param {
                            id: id.clone(),
                            name: m.argument_names.get(i).cloned().unwrap_or_default(),
                            default: m.argument_defaults.get(i).cloned().unwrap_or(Value::Str(JsStr::empty())),
                        })
                        .collect();
                    tc.proc_index.insert(m.proccode.clone(), tc.procs.len());
                    tc.procs.push(ProcInfo { params, def_block: None, warp: false });
                }
            }
        }
    }
    for (bi, b) in target.blocks.iter().enumerate() {
        if b.opcode != "procedures_definition" {
            continue;
        }
        let proto = match b.input("custom_block") {
            Some(Input::Block(id)) => target.block_index.get(id).map(|&i| &target.blocks[i]),
            _ => None,
        };
        let Some(m) = proto.and_then(|p| p.mutation.as_ref()) else { continue };
        if let Some(&pi) = tc.proc_index.get(&m.proccode) {
            if tc.procs[pi].def_block.is_none() {
                tc.procs[pi].def_block = Some(bi);
                tc.procs[pi].warp = m.warp;
            }
        }
    }

    // Procedure bodies.
    let mut procs = Vec::new();
    let codes: Vec<(String, usize)> = tc.proc_index.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let mut by_index: Vec<String> = vec![String::new(); tc.procs.len()];
    for (code, i) in codes {
        by_index[i] = code;
    }
    for pi in 0..tc.procs.len() {
        let body = match tc.procs[pi].def_block {
            Some(bi) => {
                tc.cur_params = Some(tc.procs[pi].params.iter().map(|p| p.name.clone()).collect());
                let next = target.blocks[bi].next.clone();
                let body = lower_chain(cx, &tc, next.as_deref());
                tc.cur_params = None;
                body
            }
            None => Vec::new(),
        };
        procs.push(Proc { proccode: by_index[pi].clone(), params: tc.procs[pi].params.clone(), warp: tc.procs[pi].warp, body });
    }

    // Hat scripts, in `_scripts` order (top-level blocks in block order).
    let mut scripts = Vec::new();
    for b in &target.blocks {
        if !b.top_level {
            continue;
        }
        if let Some(hat) = lower_hat(cx, &tc, b) {
            let body = lower_chain(cx, &tc, b.next.as_deref());
            scripts.push(Script { hat, body, block_id: b.id.clone() });
        }
    }
    cx.targets[ti].scripts = scripts;
    cx.targets[ti].procs = procs;
}

fn field_str(b: &Block, name: &str) -> String {
    b.field(name).map(|f| f.value.to_str().to_rust_string()).unwrap_or_default()
}

fn lower_hat(cx: &mut Ctx, tc: &TCtx, b: &Block) -> Option<Hat> {
    Some(match b.opcode.as_str() {
        "event_whenflagclicked" => Hat::FlagClicked,
        "event_whenkeypressed" => Hat::KeyPressed(js_upper(&field_str(b, "KEY_OPTION"))),
        "event_whenthisspriteclicked" => Hat::SpriteClicked,
        "event_whenstageclicked" => Hat::StageClicked,
        "event_whenbackdropswitchesto" => Hat::BackdropSwitchesTo(js_upper(&field_str(b, "BACKDROP"))),
        "event_whengreaterthan" => {
            Hat::GreaterThan { menu: js_lower(&field_str(b, "WHENGREATERTHANMENU")), value: lower_input(cx, tc, b, "VALUE", 0) }
        }
        "event_whenbroadcastreceived" => Hat::BroadcastReceived(cx.message(&field_str(b, "BROADCAST_OPTION"))),
        "control_start_as_clone" => Hat::StartAsClone,
        "event_whentouchingobject" => Hat::TouchingObject(lower_input(cx, tc, b, "TOUCHINGOBJECTMENU", 0)),
        _ => return None,
    })
}

const MAX_DEPTH: usize = 400;

/// Lower a `next` chain starting at `start`. Cycles are cut.
fn lower_chain(cx: &mut Ctx, tc: &TCtx, start: Option<&str>) -> Vec<Stmt> {
    lower_chain_d(cx, tc, start, 0)
}

fn lower_chain_d(cx: &mut Ctx, tc: &TCtx, start: Option<&str>, depth: usize) -> Vec<Stmt> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = start;
    while let Some(id) = cur {
        if !seen.insert(id) {
            break;
        }
        let Some(&bi) = tc.target.block_index.get(id) else { break };
        let b = &tc.target.blocks[bi];
        lower_stmt(cx, tc, b, depth, &mut out);
        cur = b.next.as_deref();
    }
    out
}

fn substack(cx: &mut Ctx, tc: &TCtx, b: &Block, name: &str, depth: usize) -> Vec<Stmt> {
    match b.input(name) {
        Some(Input::Block(id)) => lower_chain_d(cx, tc, Some(id.as_str()), depth + 1),
        _ => Vec::new(),
    }
}

/// Evaluation order of a block's arguments. TurboWarp's compiler evaluates
/// them in the order its generated JavaScript names them, which is the
/// argument order (scratch-vm's interpreter follows the order of the inputs
/// in project.json instead).
fn eval_order(_b: &Block, names: &[&str]) -> Vec<usize> {
    (0..names.len()).collect()
}

fn var_field(cx: &mut Ctx, tc: &TCtx, b: &Block) -> VarId {
    let f = b.field("VARIABLE");
    let name = f.map(|f| f.value.to_str().to_rust_string()).unwrap_or_default();
    let id = f.and_then(|f| f.id.clone()).unwrap_or_default();
    cx.resolve_var(tc.ti, &id, &name)
}

fn list_field(cx: &mut Ctx, tc: &TCtx, b: &Block) -> ListId {
    let f = b.field("LIST");
    let name = f.map(|f| f.value.to_str().to_rust_string()).unwrap_or_default();
    let id = f.and_then(|f| f.id.clone()).unwrap_or_default();
    cx.resolve_list(tc.ti, &id, &name)
}

fn broadcast_input(cx: &mut Ctx, tc: &TCtx, b: &Block, depth: usize) -> BroadcastRef {
    match b.input("BROADCAST_INPUT") {
        Some(Input::Broadcast { name, id }) => cx.resolve_broadcast(id.as_deref(), name),
        Some(Input::Block(bid)) => {
            if let Some(&bi) = tc.target.block_index.get(bid) {
                let sb = &tc.target.blocks[bi];
                if sb.opcode == "event_broadcast_menu" {
                    let f = sb.field("BROADCAST_OPTION");
                    let name = f.map(|f| f.value.to_str().to_rust_string()).unwrap_or_default();
                    let id = f.and_then(|f| f.id.clone());
                    return cx.resolve_broadcast(id.as_deref(), &name);
                }
            }
            BroadcastRef::Dynamic(lower_input(cx, tc, b, "BROADCAST_INPUT", depth))
        }
        Some(Input::Empty) | None => BroadcastRef::Nothing,
        Some(_) => BroadcastRef::Dynamic(lower_input(cx, tc, b, "BROADCAST_INPUT", depth)),
    }
}

fn lower_stmt(cx: &mut Ctx, tc: &TCtx, b: &Block, depth: usize, out: &mut Vec<Stmt>) {
    let op = b.opcode.as_str();
    macro_rules! inp {
        ($name:expr) => {
            lower_input(cx, tc, b, $name, depth)
        };
    }
    if let Some(cmd) = Cmd::from_opcode(op) {
        let mut args = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        for a in cmd.args() {
            match a {
                A::In(name, _) => {
                    args.push(inp!(name));
                    names.push(name);
                }
                A::Field(name) => {
                    args.push(Expr::Lit(Value::Str(JsStr::from(field_str(b, name).as_str()))));
                    names.push("");
                }
            }
        }
        let order = eval_order(b, &names);
        if cmd.is_noop() {
            out.push(Stmt::Discard(args));
        } else {
            out.push(Stmt::Cmd(cmd, args, order));
        }
        return;
    }
    let s = match op {
        "data_setvariableto" => Stmt::SetVar(var_field(cx, tc, b), inp!("VALUE")),
        "data_changevariableby" => Stmt::ChangeVar(var_field(cx, tc, b), inp!("VALUE")),
        "data_showvariable" => Stmt::ShowVar(var_field(cx, tc, b), true),
        "data_hidevariable" => Stmt::ShowVar(var_field(cx, tc, b), false),
        "data_showlist" => Stmt::ShowList(list_field(cx, tc, b), true),
        "data_hidelist" => Stmt::ShowList(list_field(cx, tc, b), false),
        "data_addtolist" => Stmt::ListAdd(list_field(cx, tc, b), inp!("ITEM")),
        "data_deleteoflist" => Stmt::ListDelete(list_field(cx, tc, b), inp!("INDEX")),
        "data_deletealloflist" => Stmt::ListDeleteAll(list_field(cx, tc, b)),
        "data_insertatlist" => Stmt::ListInsert(list_field(cx, tc, b), inp!("INDEX"), inp!("ITEM")),
        "data_replaceitemoflist" => Stmt::ListReplace(list_field(cx, tc, b), inp!("INDEX"), inp!("ITEM")),
        "control_if" => Stmt::If(inp!("CONDITION"), substack(cx, tc, b, "SUBSTACK", depth), Vec::new()),
        "control_if_else" => {
            Stmt::If(inp!("CONDITION"), substack(cx, tc, b, "SUBSTACK", depth), substack(cx, tc, b, "SUBSTACK2", depth))
        }
        "control_repeat" => Stmt::Repeat(inp!("TIMES"), substack(cx, tc, b, "SUBSTACK", depth)),
        "control_forever" => Stmt::Forever(substack(cx, tc, b, "SUBSTACK", depth)),
        "control_repeat_until" => Stmt::RepeatUntil(inp!("CONDITION"), substack(cx, tc, b, "SUBSTACK", depth)),
        "control_while" => Stmt::While(inp!("CONDITION"), substack(cx, tc, b, "SUBSTACK", depth)),
        "control_for_each" => Stmt::ForEach(var_field(cx, tc, b), inp!("VALUE"), substack(cx, tc, b, "SUBSTACK", depth)),
        // "all at once" simply runs its branch.
        "control_all_at_once" => Stmt::If(Expr::Lit(Value::Bool(true)), substack(cx, tc, b, "SUBSTACK", depth), Vec::new()),
        "control_wait" => Stmt::Wait(inp!("DURATION")),
        "music_playNoteForBeats" => Stmt::MusicWait { kind: 0, arg: inp!("NOTE"), beats: inp!("BEATS") },
        "music_playDrumForBeats" => Stmt::MusicWait { kind: 1, arg: inp!("DRUM"), beats: inp!("BEATS") },
        "music_midiPlayDrumForBeats" => Stmt::MusicWait { kind: 1, arg: inp!("DRUM"), beats: inp!("BEATS") },
        "music_restForBeats" => Stmt::MusicWait { kind: 2, arg: Expr::Missing, beats: inp!("BEATS") },
        "control_wait_until" => Stmt::WaitUntil(inp!("CONDITION")),
        "control_stop" => match field_str(b, "STOP_OPTION").as_str() {
            "all" => Stmt::StopAll,
            "this script" => Stmt::StopThisScript,
            "other scripts in sprite" | "other scripts in stage" => Stmt::StopOtherScripts,
            _ => return,
        },
        "control_delete_this_clone" => Stmt::DeleteThisClone,
        "event_broadcast" => Stmt::Broadcast(broadcast_input(cx, tc, b, depth)),
        "event_broadcastandwait" => Stmt::BroadcastAndWait(broadcast_input(cx, tc, b, depth)),
        "motion_glidesecstoxy" => Stmt::Glide { secs: inp!("SECS"), x: inp!("X"), y: inp!("Y") },
        "motion_glideto" => Stmt::GlideTo { secs: inp!("SECS"), to: inp!("TO") },
        "looks_sayforsecs" => Stmt::SayForSecs { think: false, message: inp!("MESSAGE"), secs: inp!("SECS") },
        "looks_thinkforsecs" => Stmt::SayForSecs { think: true, message: inp!("MESSAGE"), secs: inp!("SECS") },
        "looks_switchbackdroptoandwait" => Stmt::SwitchBackdropAndWait(inp!("BACKDROP")),
        "sound_playuntildone" => Stmt::PlaySoundUntilDone(inp!("SOUND_MENU")),
        "sensing_askandwait" => Stmt::Ask(inp!("QUESTION")),
        "procedures_call" => {
            let Some(m) = &b.mutation else { return };
            let ids: Vec<&str> = b.inputs.iter().map(|(n, _)| n.as_str()).collect();
            match tc.proc_index.get(&m.proccode) {
                // TurboWarp compiles a call to an empty definition to nothing,
                // without evaluating the arguments (jsgen.js).
                Some(&pi) if tc.procs[pi].def_block.is_some_and(|bi| tc.target.blocks[bi].next.is_none()) => return,
                Some(&pi) if tc.procs[pi].def_block.is_some() => {
                    let params = &tc.procs[pi].params;
                    let mut args = Vec::new();
                    let names: Vec<&str> = params.iter().map(|p| p.id.as_str()).collect();
                    for p in params {
                        let present = matches!(b.input(&p.id), Some(i) if !matches!(i, Input::Empty));
                        if present {
                            args.push(inp!(&p.id));
                        } else {
                            args.push(Expr::Lit(p.default.clone()));
                        }
                    }
                    let order = eval_order(b, &names);
                    Stmt::Call { proc: ProcId { target: tc.ti, index: pi }, args, order }
                }
                _ => {
                    // No prototype or no definition: TurboWarp emits a no-op and
                    // does not evaluate the inputs.
                    let _ = ids;
                    return;
                }
            }
        }
        // Blocks that are valid but do nothing when reached in a stack.
        "procedures_definition" | "procedures_prototype" => return,
        _ => {
            if is_hat_opcode(op) {
                return;
            }
            // A reporter dropped into a stack position cannot happen; anything
            // else is an unsupported command (extension or unknown opcode).
            cx.unsupported.push((tc.target.name.clone(), op.to_string()));
            let names: Vec<String> = b.inputs.iter().filter(|(n, _)| !n.starts_with("SUBSTACK")).map(|(n, _)| n.clone()).collect();
            let args: Vec<Expr> = names.iter().map(|n| inp!(n)).collect();
            Stmt::Discard(args)
        }
    };
    out.push(s);
}

fn is_hat_opcode(op: &str) -> bool {
    matches!(
        op,
        "event_whenflagclicked"
            | "event_whenkeypressed"
            | "event_whenthisspriteclicked"
            | "event_whenstageclicked"
            | "event_whenbackdropswitchesto"
            | "event_whengreaterthan"
            | "event_whenbroadcastreceived"
            | "control_start_as_clone"
            | "event_whentouchingobject"
    )
}

fn lower_input(cx: &mut Ctx, tc: &TCtx, b: &Block, name: &str, depth: usize) -> Expr {
    match b.input(name) {
        None | Some(Input::Empty) => Expr::Missing,
        Some(Input::Literal(v)) => Expr::Lit(v.clone()),
        Some(Input::Broadcast { name, .. }) => Expr::Lit(Value::Str(JsStr::from(name.as_str()))),
        Some(Input::Variable { name, id }) => Expr::Var(cx.resolve_var(tc.ti, id, name)),
        Some(Input::List { name, id }) => Expr::ListContents(cx.resolve_list(tc.ti, id, name)),
        Some(Input::Block(id)) => match tc.target.block_index.get(id) {
            Some(&bi) if depth < MAX_DEPTH => lower_reporter(cx, tc, &tc.target.blocks[bi], depth + 1),
            _ => Expr::Missing,
        },
    }
}

fn binop(cx: &mut Ctx, tc: &TCtx, b: &Block, op: Op, n1: &str, n2: &str, depth: usize) -> Expr {
    let a = lower_input(cx, tc, b, n1, depth);
    let c = lower_input(cx, tc, b, n2, depth);
    // `letter of` compiles to `(STRING)[LETTER - 1]`: the string goes first.
    let order = if op == Op::LetterOf && a.is_impure() && c.is_impure() { Some(vec![1, 0]) } else { None };
    let _ = b;
    Expr::Op(op, vec![a, c], order)
}

fn lower_reporter(cx: &mut Ctx, tc: &TCtx, b: &Block, depth: usize) -> Expr {
    let op = b.opcode.as_str();
    macro_rules! inp {
        ($name:expr) => {
            lower_input(cx, tc, b, $name, depth)
        };
    }
    match op {
        "operator_add" => binop(cx, tc, b, Op::Add, "NUM1", "NUM2", depth),
        "operator_subtract" => binop(cx, tc, b, Op::Sub, "NUM1", "NUM2", depth),
        "operator_multiply" => binop(cx, tc, b, Op::Mul, "NUM1", "NUM2", depth),
        "operator_divide" => binop(cx, tc, b, Op::Div, "NUM1", "NUM2", depth),
        "operator_mod" => binop(cx, tc, b, Op::Mod, "NUM1", "NUM2", depth),
        "operator_random" => binop(cx, tc, b, Op::Random, "FROM", "TO", depth),
        "operator_lt" => binop(cx, tc, b, Op::Lt, "OPERAND1", "OPERAND2", depth),
        "operator_equals" => binop(cx, tc, b, Op::Eq, "OPERAND1", "OPERAND2", depth),
        "operator_gt" => binop(cx, tc, b, Op::Gt, "OPERAND1", "OPERAND2", depth),
        "operator_and" => binop(cx, tc, b, Op::And, "OPERAND1", "OPERAND2", depth),
        "operator_or" => binop(cx, tc, b, Op::Or, "OPERAND1", "OPERAND2", depth),
        "operator_join" => binop(cx, tc, b, Op::Join, "STRING1", "STRING2", depth),
        "operator_letter_of" => binop(cx, tc, b, Op::LetterOf, "LETTER", "STRING", depth),
        "operator_contains" => binop(cx, tc, b, Op::Contains, "STRING1", "STRING2", depth),
        "operator_not" => Expr::Op(Op::Not, vec![inp!("OPERAND")], None),
        "operator_length" => Expr::Op(Op::Length, vec![inp!("STRING")], None),
        "operator_round" => Expr::Op(Op::Round, vec![inp!("NUM")], None),
        "operator_mathop" => {
            let name = b.field("OPERATOR").map(|f| f.value.to_str()).unwrap_or_default();
            Expr::Op(Op::Math(MathOp::from_name(&name)), vec![inp!("NUM")], None)
        }
        "data_variable" => Expr::Var(var_field(cx, tc, b)),
        "data_listcontents" => Expr::ListContents(list_field(cx, tc, b)),
        "data_itemoflist" => Expr::ListItem(list_field(cx, tc, b), Box::new(inp!("INDEX"))),
        "data_itemnumoflist" => Expr::ListIndexOf(list_field(cx, tc, b), Box::new(inp!("ITEM"))),
        "data_lengthoflist" => Expr::ListLen(list_field(cx, tc, b)),
        "data_listcontainsitem" => Expr::ListContains(list_field(cx, tc, b), Box::new(inp!("ITEM"))),
        "argument_reporter_string_number" | "argument_reporter_boolean" => {
            let name = field_str(b, "VALUE");
            // The last parameter with that name wins.
            let found = tc.cur_params.as_ref().and_then(|params| params.iter().rposition(|p| *p == name));
            match found {
                Some(i) => Expr::Arg(i),
                // TurboWarp (irgen.js): an unbound boolean argument named
                // "is compiled?" or "is TurboWarp?" is `true`.
                None if op == "argument_reporter_boolean" && matches!(name.to_lowercase().as_str(), "is compiled?" | "is turbowarp?") => Expr::Lit(Value::Bool(true)),
                None => Expr::num(0.0),
            }
        }
        "looks_costumenumbername" => {
            if field_str(b, "NUMBER_NAME") == "number" {
                Expr::Rep(Rep::CostumeNumber, vec![])
            } else {
                Expr::Rep(Rep::CostumeName, vec![])
            }
        }
        "looks_backdropnumbername" => {
            if field_str(b, "NUMBER_NAME") == "number" {
                Expr::Rep(Rep::BackdropNumber, vec![])
            } else {
                Expr::Rep(Rep::BackdropName, vec![])
            }
        }
        "sensing_of" => lower_sensing_of(cx, tc, b, depth),
        _ => {
            if let Some(rep) = Rep::from_opcode(op) {
                let args = rep
                    .args()
                    .iter()
                    .map(|a| match a {
                        A::In(name, _) => inp!(name),
                        A::Field(name) => Expr::Lit(Value::Str(JsStr::from(field_str(b, name).as_str()))),
                    })
                    .collect();
                return Expr::Rep(rep, args);
            }
            // Shadow / menu blocks: no primitive, exactly one field, no inputs.
            if b.fields.len() == 1 && b.inputs.is_empty() && !is_known_function_opcode(op) {
                return Expr::Lit(b.fields[0].1.value.clone());
            }
            cx.unsupported.push((tc.target.name.clone(), op.to_string()));
            Expr::Missing
        }
    }
}

/// Opcodes that have a primitive in scratch-vm even though they look like a
/// one-field block (so they are not "shadow values").
fn is_known_function_opcode(op: &str) -> bool {
    matches!(op, "data_variable" | "data_listcontents" | "sensing_current" | "argument_reporter_string_number" | "argument_reporter_boolean")
}

fn lower_sensing_of(cx: &mut Ctx, tc: &TCtx, b: &Block, depth: usize) -> Expr {
    let property = field_str(b, "PROPERTY");
    let object = lower_input(cx, tc, b, "OBJECT", depth);
    let generic = |object: Expr| Expr::Rep(Rep::AttrOf, vec![Expr::Lit(Value::Str(JsStr::from(property.as_str()))), object]);
    let Expr::Lit(obj) = &object else { return generic(object) };
    // `args.OBJECT === '_stage_'` is a strict comparison on the raw value.
    let target = match obj {
        Value::Str(s) if s.eq_str("_stage_") => Some(0),
        other => {
            let name = other.to_str().to_rust_string();
            cx.project.targets.iter().position(|t| !t.is_stage && t.name == name)
        }
    };
    let Some(t) = target else { return Expr::num(0.0) };
    let builtin: &[&str] = if t == 0 {
        &["background #", "backdrop #", "backdrop name", "volume"]
    } else {
        &["x position", "y position", "direction", "costume #", "costume name", "size", "volume"]
    };
    if builtin.contains(&property.as_str()) {
        return generic(object);
    }
    // lookupVariableByNameAndType(name, '', skipStage = true)
    match cx.targets[t].vars.iter().position(|v| v.name == property) {
        Some(i) => Expr::VarOf(VarId { target: t, index: i }),
        None => Expr::num(0.0),
    }
}
