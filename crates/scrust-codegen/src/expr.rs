//! Expression code generation: typed Rust expressions plus the casts
//! (`Cast.toNumber` & co.) resolved statically wherever the type allows.

use crate::Gen;
use scrust_analysis::{fold, Node, Owner, Repr, Ty};
use scrust_ir::*;
use scrust_value::{JsStr, ToBool, ToNum, ToStr, Value};

/// Rust-level type of a generated expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RT {
    Num,
    Str,
    Bool,
    Val,
}

/// A generated expression. `code` always evaluates to an owned value.
#[derive(Clone, Debug)]
pub struct E {
    pub code: String,
    pub rt: RT,
    pub ty: Ty,
    /// Compile-time constant, if known.
    pub lit: Option<Value>,
    /// A borrowable place holding the value (variable storage, argument).
    pub place: Option<String>,
    /// An empty input slot.
    pub missing: bool,
    /// Typed accessors for list items: (as number, as string).
    pub alt: Option<(String, String)>,
}

pub struct Cx {
    pub owner: Owner,
    pub ti: usize,
}

pub fn f64_lit(x: f64) -> String {
    if x.is_nan() {
        "f64::NAN".into()
    } else if x == f64::INFINITY {
        "f64::INFINITY".into()
    } else if x == f64::NEG_INFINITY {
        "f64::NEG_INFINITY".into()
    } else {
        let s = format!("{:?}", x);
        if x.is_sign_negative() {
            format!("({}_f64)", s)
        } else {
            format!("{}_f64", s)
        }
    }
}

pub fn str_lit(s: &JsStr) -> String {
    let units = s.to_utf16();
    match String::from_utf16(&units) {
        Ok(text) => {
            if text.is_empty() {
                "JsStr::empty()".into()
            } else {
                format!("jstr!({:?})", text)
            }
        }
        // Lone surrogates cannot be written as a Rust string literal.
        Err(_) => format!("JsStr::from_utf16(&{:?})", units),
    }
}

pub fn val_lit(v: &Value) -> String {
    match v {
        Value::Num(n) => format!("Value::Num({})", f64_lit(*n)),
        Value::Str(s) => format!("Value::Str({})", str_lit(s)),
        Value::Bool(b) => format!("Value::Bool({})", b),
    }
}

impl E {
    pub fn lit(v: Value) -> E {
        let (code, rt) = match &v {
            Value::Num(n) => (f64_lit(*n), RT::Num),
            Value::Str(s) => (str_lit(s), RT::Str),
            Value::Bool(b) => (b.to_string(), RT::Bool),
        };
        E { code, rt, ty: Ty::of_value(&v), lit: Some(v), place: None, missing: false, alt: None }
    }
    pub fn new(code: String, rt: RT, ty: Ty) -> E {
        E { code, rt, ty, lit: None, place: None, missing: false, alt: None }
    }
    fn uses_world(&self) -> bool {
        self.code.contains("w.")
    }

    /// `Cast.toNumber`
    pub fn num(&self) -> String {
        if self.missing {
            return "0.0_f64".into();
        }
        if let Some(v) = &self.lit {
            return f64_lit(v.to_num());
        }
        if let Some((n, _)) = &self.alt {
            return n.clone();
        }
        match self.rt {
            RT::Num => {
                if self.ty.has(Ty::NAN) {
                    format!("nn({})", self.code)
                } else {
                    self.code.clone()
                }
            }
            RT::Bool => format!("(({}) as u8 as f64)", self.code),
            RT::Str | RT::Val => match &self.place {
                Some(p) => format!("{}.to_num()", p),
                None => format!("({}).to_num()", self.code),
            },
        }
    }

    /// Raw JS `Number(x)`: NaN is preserved.
    pub fn js_number(&self) -> String {
        if self.missing {
            return "f64::NAN".into();
        }
        if let Some(v) = &self.lit {
            return f64_lit(v.js_number());
        }
        match self.rt {
            RT::Num => self.code.clone(),
            RT::Bool => format!("(({}) as u8 as f64)", self.code),
            RT::Str | RT::Val => match &self.place {
                Some(p) => format!("{}.js_number()", p),
                None => format!("({}).js_number()", self.code),
            },
        }
    }

    /// `Cast.toString`
    pub fn str(&self) -> String {
        if self.missing {
            return "jstr!(\"undefined\")".into();
        }
        if let Some(v) = &self.lit {
            return str_lit(&v.to_str());
        }
        if let Some((_, s)) = &self.alt {
            return s.clone();
        }
        match self.rt {
            RT::Str => self.code.clone(),
            RT::Num => format!("js_num_to_str({})", self.code),
            RT::Bool => format!("({}).to_str()", self.code),
            RT::Val => match &self.place {
                Some(p) => format!("{}.to_str()", p),
                None => format!("({}).to_str()", self.code),
            },
        }
    }

    /// `Cast.toBoolean`
    pub fn bool(&self) -> String {
        if self.missing {
            return "false".into();
        }
        if let Some(v) = &self.lit {
            return v.to_bool().to_string();
        }
        match self.rt {
            RT::Bool => self.code.clone(),
            RT::Num => format!("({}).to_bool()", self.code),
            RT::Str | RT::Val => match &self.place {
                Some(p) => format!("{}.to_bool()", p),
                None => format!("({}).to_bool()", self.code),
            },
        }
    }

    /// The raw value as a dynamic `Value`.
    pub fn val(&self) -> String {
        if self.missing {
            return "Value::Str(jstr!(\"undefined\"))".into();
        }
        if let Some(v) = &self.lit {
            return val_lit(v);
        }
        match self.rt {
            RT::Val => self.code.clone(),
            RT::Num => format!("Value::Num({})", self.code),
            RT::Str => format!("Value::Str({})", self.code),
            RT::Bool => format!("Value::Bool({})", self.code),
        }
    }

    /// The raw value converted to a storage representation chosen by the
    /// analysis (which guarantees the conversion is lossless or unobservable).
    pub fn as_repr(&self, r: Repr) -> String {
        match r {
            Repr::Val => self.val(),
            Repr::F64 => {
                if let Some(v) = &self.lit {
                    return f64_lit(v.js_number());
                }
                match self.rt {
                    RT::Num => self.code.clone(),
                    _ => format!("<f64 as Elem>::from_value(&{})", self.val()),
                }
            }
            Repr::Str => self.str(),
            Repr::Bool => match (&self.lit, self.rt) {
                (Some(Value::Bool(b)), _) => b.to_string(),
                (_, RT::Bool) => self.code.clone(),
                _ => format!("<bool as Elem>::from_value(&{})", self.val()),
            },
        }
    }

    /// `Cast.isInt(raw)`
    fn is_int(&self) -> String {
        if self.missing {
            return "false".into();
        }
        if let Some(v) = &self.lit {
            return scrust_value::is_int(v).to_string();
        }
        match self.rt {
            RT::Num => format!("scrust_runtime::value::num::js_is_int_number({})", self.code),
            RT::Bool => "true".into(),
            _ => format!("scrust_runtime::value::is_int(&{})", self.val()),
        }
    }
}

fn rt_of(r: Repr) -> RT {
    match r {
        Repr::F64 => RT::Num,
        Repr::Bool => RT::Bool,
        Repr::Str => RT::Str,
        Repr::Val => RT::Val,
    }
}

impl<'a> Gen<'a> {
    /// Storage place of a variable. `write` selects exclusive access.
    pub fn var_place(&self, v: VarId, cx: &Cx, of_original: bool, write: bool) -> String {
        let acc = if write { "m" } else { "r" };
        if v.target == 0 {
            format!("g.{}().v{}", acc, v.index)
        } else if of_original || v.target != cx.ti {
            format!("w.locals_of::<L{}>({}).v{}", v.target, v.target, v.index)
        } else {
            format!("l.{}().v{}", acc, v.index)
        }
    }

    pub fn list_place(&self, l: ListId, _cx: &Cx, write: bool) -> String {
        let acc = if write { "m" } else { "r" };
        if l.target == 0 {
            format!("g.{}().l{}", acc, l.index)
        } else {
            format!("l.{}().l{}", acc, l.index)
        }
    }

    fn storage_read(&self, place: String, n: Node) -> E {
        let repr = self.an.repr(n);
        let ty = match self.an.read_ty(n) {
            t if t.is_empty() => Ty::ANY,
            t => t,
        };
        let rt = rt_of(repr);
        match repr {
            Repr::F64 | Repr::Bool => E::new(place, rt, ty),
            Repr::Str | Repr::Val => E { code: format!("{}.clone()", place), rt, ty, lit: None, place: Some(place), missing: false, alt: None },
        }
    }

    /// Code computing a `ListIndex` for `idx` against the list at `place`.
    /// Returns (prelude statements, index expression).
    pub fn list_index(&mut self, idx: &E, place: &str, len_plus: usize, accept_all: bool) -> (String, String) {
        let len = if len_plus == 0 { format!("{}.len()", place) } else { format!("{}.len() + {}", place, len_plus) };
        let numeric = idx.rt == RT::Num || matches!(idx.lit, Some(Value::Num(_)));
        if numeric && idx.lit.is_none() || matches!(idx.lit, Some(Value::Num(_))) {
            (format!("let i_ = {}; let n_ = {};", idx.num(), len), "num_to_list_index(i_, n_)".into())
        } else {
            (format!("let i_ = {}; let n_ = {};", idx.val(), len), format!("w.list_index(&i_, n_, {})", accept_all))
        }
    }

    pub fn expr(&mut self, e: &Expr, cx: &Cx) -> E {
        if let Some(v) = fold(e) {
            return E::lit(v);
        }
        match e {
            Expr::Lit(v) => E::lit(v.clone()),
            Expr::Missing => E { code: "Value::Str(jstr!(\"undefined\"))".into(), rt: RT::Val, ty: Ty::STR_NAN, lit: None, place: None, missing: true, alt: None },
            Expr::Var(v) => {
                let place = self.var_place(*v, cx, false, false);
                // Flow type (§8.5): a dynamically stored variable whose
                // current contents are statically known.
                if let Some(&ty) = self.flow.get(v) {
                    if (v.target == 0 || v.target == cx.ti) && self.an.repr(Node::Var(*v)) == Repr::Val {
                        if ty.within(Ty::NUMBER_OR_NAN) {
                            return E::new(format!("val_num(&{})", place), RT::Num, ty);
                        }
                        if ty.within(Ty::STRING) {
                            return E::new(format!("val_str(&{})", place), RT::Str, ty);
                        }
                        if ty.within(Ty::BOOLEAN) {
                            return E::new(format!("val_bool(&{})", place), RT::Bool, ty);
                        }
                        let mut e = self.storage_read(place, Node::Var(*v));
                        e.ty = ty;
                        return e;
                    }
                }
                self.storage_read(place, Node::Var(*v))
            }
            Expr::VarOf(v) => self.storage_read(self.var_place(*v, cx, true, false), Node::Var(*v)),
            Expr::Arg(i) => match cx.owner.proc() {
                Some(p) => self.storage_read(format!("a{}", i), Node::Param(p, *i)),
                None => E::lit(Value::Num(0.0)),
            },
            Expr::ListContents(l) => E::new(format!("{}.contents()", self.list_place(*l, cx, false)), RT::Str, Ty::STRING),
            Expr::ListLen(l) => E::new(format!("({}.len() as f64)", self.list_place(*l, cx, false)), RT::Num, Ty::POS_INT | Ty::ZERO),
            Expr::ListItem(l, idx) => {
                let place = self.list_place(*l, cx, false);
                let i = self.expr(idx, cx);
                let (pre, ix) = self.list_index(&i, &place, 0, false);
                let ty = self.an.ty_of(e, cx.owner);
                let mk = |m: &str| format!("{{ {} let ix_ = {}; {}.{}(ix_) }}", pre, ix, place, m);
                E { code: mk("item"), rt: RT::Val, ty, lit: None, place: None, missing: false, alt: Some((mk("item_num"), mk("item_str"))) }
            }
            Expr::ListIndexOf(l, item) => {
                let place = self.list_place(*l, cx, false);
                let v = self.expr(item, cx);
                E::new(format!("{{ let v_ = {}; {}.index_of(&v_) }}", v.val(), place), RT::Num, Ty::POS_INT | Ty::ZERO)
            }
            Expr::ListContains(l, item) => {
                let place = self.list_place(*l, cx, false);
                let v = self.expr(item, cx);
                E::new(format!("{{ let v_ = {}; {}.contains(&v_) }}", v.val(), place), RT::Bool, Ty::BOOLEAN)
            }
            Expr::Rep(rep, args) => {
                let mut lets = String::new();
                let mut names = Vec::new();
                for (i, (spec, a)) in rep.args().iter().zip(args).enumerate() {
                    let code = match spec {
                        A::Field(_) => match a {
                            Expr::Lit(v) => format!("{:?}", v.to_str().to_rust_string()),
                            _ => "\"\"".into(),
                        },
                        A::In(_, k) => {
                            let x = self.expr(a, cx);
                            cast(&x, *k)
                        }
                    };
                    lets.push_str(&format!("let r{} = {}; ", i, code));
                    names.push(format!(", r{}", i));
                }
                let rt = match rep.result() {
                    R::Num => RT::Num,
                    R::Bool => RT::Bool,
                    R::Str => RT::Str,
                    R::Val => RT::Val,
                };
                let call = format!("w.{}(me{})", rep.method(), names.concat());
                let code = if lets.is_empty() { call } else { format!("{{ {}{} }}", lets, call) };
                E::new(code, rt, self.an.ty_of(e, cx.owner))
            }
            Expr::Op(op, args, order) => self.op(e, *op, args, order.as_deref(), cx),
        }
    }

    fn op(&mut self, e: &Expr, op: Op, args: &[Expr], order: Option<&[usize]>, cx: &Cx) -> E {
        let ty = self.an.ty_of(e, cx.owner);
        // TurboWarp folds `pick random` between two equal literal inputs
        // (irgen.js) without drawing a random number.
        if let (Op::Random, [Expr::Lit(a), Expr::Lit(b)]) = (op, args) {
            if a.to_num() == b.to_num() {
                return E::lit(Value::Num(a.to_num()));
            }
        }
        let xs: Vec<E> = args.iter().map(|a| self.expr(a, cx)).collect();
        // A non-default evaluation order only matters when it was recorded
        // (both operands consume randomness): evaluate into temporaries.
        if let Some(order) = order {
            let mut lets = String::new();
            let mut tmp: Vec<E> = xs.clone();
            for &i in order {
                lets.push_str(&format!("let o{} = {}; ", i, xs[i].val()));
                tmp[i] = E { code: format!("o{}.clone()", i), rt: RT::Val, ty: xs[i].ty, lit: None, place: Some(format!("o{}", i)), missing: false, alt: None };
            }
            let inner = self.op_code(op, &tmp, ty);
            return E::new(format!("{{ {}{} }}", lets, inner.code), inner.rt, ty);
        }
        self.op_code(op, &xs, ty)
    }

    fn op_code(&mut self, op: Op, xs: &[E], ty: Ty) -> E {
        let num2 = |sym: &str| E::new(format!("({} {} {})", xs[0].num(), sym, xs[1].num()), RT::Num, ty);
        match op {
            Op::Add => num2("+"),
            Op::Sub => num2("-"),
            Op::Mul => num2("*"),
            Op::Div => num2("/"),
            Op::Mod => E::new(format!("ops::js_mod({}, {})", xs[0].num(), xs[1].num()), RT::Num, ty),
            Op::Round => E::new(format!("js_round({})", xs[0].num()), RT::Num, ty),
            Op::Lt | Op::Eq | Op::Gt => {
                let sym = match op {
                    Op::Lt => "<",
                    Op::Eq => "==",
                    _ => ">",
                };
                // Numeric view of an operand: (code, may be NaN, NaN-safe).
                // "NaN-safe" means that if the *other* side is NaN, the string
                // comparison scratch-vm falls back to orders this operand
                // before "nan" - true for every number and for numeric string
                // literals that do not start with exotic whitespace (§9.3).
                let numeric = |x: &E| -> Option<(String, bool, bool)> {
                    if x.missing {
                        return None;
                    }
                    match &x.lit {
                        Some(Value::Num(n)) => return Some((f64_lit(*n), n.is_nan(), true)),
                        Some(Value::Bool(b)) => return Some((f64_lit(*b as u8 as f64), false, false)),
                        Some(v @ Value::Str(s)) if x.ty.within(Ty::STR_CANON | Ty::STR_NUM) => {
                            let first = s.to_lower().code_unit(0);
                            return Some((f64_lit(v.js_number()), false, first < b'n' as u16));
                        }
                        Some(_) => return None,
                        None => {}
                    }
                    match x.rt {
                        RT::Num => Some((x.code.clone(), x.ty.has(Ty::NAN), true)),
                        RT::Bool => Some((format!("(({}) as u8 as f64)", x.code), false, false)),
                        _ => None,
                    }
                };
                // A constant that is certainly not numeric forces the
                // case-insensitive string comparison.
                let text_const = |x: &E| !x.missing && matches!(&x.lit, Some(Value::Str(_))) && x.ty.within(Ty::STR_NAN | Ty::STR_BOOL);
                let generic = |xs: &[E]| {
                    let usable = |x: &E, other: &E| match &x.place {
                        Some(p) if x.rt == RT::Val && (!p.contains("w.") || !other.uses_world()) => Some(format!("&{}", p)),
                        _ => None,
                    };
                    let a = usable(&xs[0], &xs[1]).unwrap_or_else(|| format!("&{}", xs[0].val()));
                    let b = usable(&xs[1], &xs[0]).unwrap_or_else(|| format!("&{}", xs[1].val()));
                    format!("(compare({}, {}) {} 0.0)", a, b, sym)
                };
                let code = match (numeric(&xs[0]), numeric(&xs[1])) {
                    (Some((a, false, _)), Some((b, false, _))) => format!("({} {} {})", a, sym, b),
                    (Some((a, _, true)), Some((b, _, true))) => {
                        let f = match op {
                            Op::Lt => "num_lt",
                            Op::Eq => "num_eq",
                            _ => "num_gt",
                        };
                        format!("{}({}, {})", f, a, b)
                    }
                    _ if text_const(&xs[0]) || text_const(&xs[1]) => {
                        format!("(compare_str_ci(&{}, &{}) {} 0.0)", xs[0].str(), xs[1].str(), sym)
                    }
                    _ => generic(xs),
                };
                E::new(code, RT::Bool, Ty::BOOLEAN)
            }
            Op::And | Op::Or => {
                let sym = if op == Op::And { "&&" } else { "||" };
                // TurboWarp compiles these to JavaScript's `&&` / `||`, so the
                // right operand is only evaluated when needed (observable
                // through `pick random`).
                let code = format!("({} {} {})", xs[0].bool(), sym, xs[1].bool());
                E::new(code, RT::Bool, Ty::BOOLEAN)
            }
            Op::Not => E::new(format!("(!{})", xs[0].bool()), RT::Bool, Ty::BOOLEAN),
            Op::Join => E::new(format!("ops::join(&{}, &{})", xs[0].str(), xs[1].str()), RT::Str, Ty::STRING),
            Op::LetterOf => E::new(format!("ops::letter_of({}, &{})", xs[0].num(), xs[1].str()), RT::Str, ty),
            Op::Length => E::new(format!("({}.len() as f64)", xs[0].str()), RT::Num, ty),
            Op::Contains => E::new(format!("ops::contains(&{}, &{})", xs[0].str(), xs[1].str()), RT::Bool, Ty::BOOLEAN),
            Op::Random => {
                let (a, b) = (&xs[0], &xs[1]);

                let simple = |x: &E| x.lit.is_some() || x.rt == RT::Num || x.rt == RT::Bool;
                let code = if simple(a) && simple(b) {
                    // Raw values are bound once: `isInt` looks at the uncast operands.
                    let bind = |x: &E, n: &str| -> (String, E) {
                        if x.lit.is_some() {
                            (String::new(), x.clone())
                        } else {
                            (format!("let {} = {}; ", n, x.code), E { code: n.to_string(), ..x.clone() })
                        }
                    };
                    let (la, ea) = bind(a, "f_");
                    let (lb, eb) = bind(b, "t_");
                    format!("{{ {}{}let i_ = {} && {}; w.random_range({}, {}, i_) }}", la, lb, ea.is_int(), eb.is_int(), ea.num(), eb.num())
                } else {
                    format!("{{ let f_ = {}; let t_ = {}; w.random_values(&f_, &t_) }}", a.val(), b.val())
                };
                E::new(code, RT::Num, ty)
            }
            Op::Math(m) => E::new(format!("MathOp::{:?}.apply({})", m, xs[0].num()), RT::Num, ty),
            Op::MathDyn => E::new(format!("ops::mathop(&{}, {})", xs[0].str(), xs[1].num()), RT::Num, ty),
        }
    }
}

/// Apply the cast an argument slot asks for.
pub fn cast(x: &E, k: K) -> String {
    match k {
        K::Num => x.num(),
        K::Str => x.str(),
        K::Bool => x.bool(),
        K::Raw | K::RawKind => x.val(),
    }
}
