//! `scrust`: compile Scratch 3 projects (.sb3) to Rust.

use scrust_analysis::Node;
use scrust_codegen::frontend::{self, tw_framerate, unsupported_report, Compiled};
use scrust_codegen::GenOptions;
use scrust_ir::{ListId, VarId};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
scrust - Scratch 3 -> Rust ahead-of-time compiler

USAGE:
    scrust build <in.sb3> [-o out/] [--target native|wasm] [--release] [options]
    scrust run   <in.sb3> [options] [-- program args]
    scrust emit  <in.sb3> -o crate/ [options]
    scrust check <in.sb3> [--types]
    scrust trace <in.sb3> --frames N [options]  (--interp: run the IR interpreter instead of compiling)

OPTIONS:
    --compat=default|strict   strict enables the warp timer
    --warp-timer              yield from warp loops after 500 ms
    --fps N                   frames per second (default: the project's TurboWarp setting, else 30)
    --turbo                   do not stop a frame on redraw requests
    --deterministic --seed N  virtual clock, seeded RNG
    --no-unbox                keep every variable as a dynamic Value
    --allow-unsupported       build even if unsupported blocks are present
    --headless                build without a window (trace always is)
    --max-passes N            scheduler passes per frame in deterministic mode
    --answers 'a|b'           scripted answers for ask-and-wait (headless)
    --debug                   build the generated crate without optimisations
    --single-html             (wasm) embed the .wasm into index.html
    --assets external         load assets from an assets/ directory next to the executable
";

#[derive(Default)]
struct Cli {
    cmd: String,
    input: Option<PathBuf>,
    out: Option<PathBuf>,
    target: String,
    release: bool,
    warp_timer: bool,
    fps: Option<f64>,
    turbo: bool,
    deterministic: bool,
    seed: Option<u32>,
    no_unbox: bool,
    allow_unsupported: bool,
    headless: bool,
    types: bool,
    frames: Option<u64>,
    max_passes: Option<u32>,
    answers: Option<String>,
    debug: bool,
    single_html: bool,
    interp: bool,
    external_assets: bool,
    passthrough: Vec<String>,
}

fn parse() -> Result<Cli, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut c = Cli { target: "native".into(), release: true, ..Default::default() };
    let mut i = 0;
    let mut positional: Vec<String> = Vec::new();
    while i < argv.len() {
        let a = argv[i].clone();
        let val = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            argv.get(*i).cloned().ok_or_else(|| format!("missing value for {a}"))
        };
        match a.as_str() {
            "-o" | "--out" => c.out = Some(PathBuf::from(val(&mut i)?)),
            "--target" => c.target = val(&mut i)?,
            "--release" => c.release = true,
            "--debug" => {
                c.release = false;
                c.debug = true;
            }
            "--compat=strict" => c.warp_timer = true,
            "--compat=default" => {}
            "--warp-timer" => c.warp_timer = true,
            "--fps" => c.fps = Some(val(&mut i)?.parse().map_err(|_| "bad --fps")?),
            "--turbo" => c.turbo = true,
            "--deterministic" => c.deterministic = true,
            "--seed" => c.seed = Some(val(&mut i)?.parse().map_err(|_| "bad --seed")?),
            "--no-unbox" => c.no_unbox = true,
            "--interp" => c.interp = true,
            "--allow-unsupported" => c.allow_unsupported = true,
            "--headless" => c.headless = true,
            "--types" => c.types = true,
            "--frames" => c.frames = Some(val(&mut i)?.parse().map_err(|_| "bad --frames")?),
            "--max-passes" => c.max_passes = Some(val(&mut i)?.parse().map_err(|_| "bad --max-passes")?),
            "--answers" => c.answers = Some(val(&mut i)?),
            "--single-html" => c.single_html = true,
            "--assets" => c.external_assets = val(&mut i)? == "external",
            "--" => {
                c.passthrough = argv[i + 1..].to_vec();
                break;
            }
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n\n{USAGE}")),
            _ => positional.push(a),
        }
        i += 1;
    }
    let mut it = positional.into_iter();
    c.cmd = it.next().ok_or_else(|| USAGE.to_string())?;
    c.input = it.next().map(PathBuf::from);
    Ok(c)
}

fn crates_dir() -> PathBuf {
    if let Ok(d) = std::env::var("SCRUST_CRATES_DIR") {
        return PathBuf::from(d);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn compile(c: &Cli) -> Result<Compiled, String> {
    let input = c.input.as_ref().ok_or("missing input .sb3")?;
    let bytes = std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
    frontend::compile(&bytes, c.no_unbox, c.warp_timer)
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() || s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("p_{s}")
    } else {
        s
    }
}

/// Write the generated crate into `dir`. Files are only rewritten when their
/// contents change so that cargo can reuse previous builds.
fn emit(c: &Cli, comp: &Compiled, dir: &Path, headless: bool) -> Result<String, String> {
    let stem = c.input.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let wasm = c.target == "wasm";
    let crate_name = format!(
        "{}{}{}{}",
        sanitize(&stem),
        if headless { "_headless" } else { "" },
        if c.no_unbox { "_nounbox" } else { "" },
        if wasm { "_web" } else { "" }
    );
    let opts = GenOptions {
        crate_name: crate_name.clone(),
        crates_dir: crates_dir().to_string_lossy().to_string(),
        headless,
        warp_timer: c.warp_timer,
        opt_level: 3,
        codegen_units: 16,
        fps: c.fps.unwrap_or_else(|| tw_framerate(&comp.project)),
        turbo: c.turbo || comp.project.tw_config().turbo,
        fencing: comp.project.tw_config().fencing,
        misc_limits: comp.project.tw_config().misc_limits,
        max_clones: comp.project.tw_config().max_clones,
        hq: comp.project.tw_config().hq,
        stage_width: comp.project.tw_config().width.unwrap_or(480.0),
        stage_height: comp.project.tw_config().height.unwrap_or(360.0),
        wasm,
        external_assets: c.external_assets,
        max_fn_lines: std::env::var("SCRUST_MAX_FN_LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000),
    };
    let out = scrust_codegen::generate(&comp.project, &comp.program, &comp.analysis, &opts);
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    // A directory may be reused between native and wasm layouts.
    let stale = dir.join("src").join(if wasm { "main.rs" } else { "lib.rs" });
    let _ = std::fs::remove_file(stale);
    for (path, contents) in &out.files {
        let full = dir.join(path);
        if std::fs::read_to_string(&full).ok().as_deref() != Some(contents.as_str()) {
            std::fs::write(&full, contents).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
        }
    }
    if !out.assets.is_empty() {
        let adir = dir.join("assets");
        std::fs::create_dir_all(&adir).map_err(|e| e.to_string())?;
        for name in &out.assets {
            let data = comp.project.assets.get(name).cloned().unwrap_or_default();
            let full = adir.join(name);
            if std::fs::read(&full).ok().as_deref() != Some(data.as_slice()) {
                std::fs::write(&full, &data).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(crate_name)
}

fn gen_root() -> PathBuf {
    crates_dir().parent().map(|p| p.join("target").join("scrust-gen")).unwrap_or_else(|| PathBuf::from("target/scrust-gen"))
}

fn cargo_build(c: &Cli, dir: &Path, crate_name: &str) -> Result<PathBuf, String> {
    let target_dir = gen_root().join("target");
    let wasm = c.target == "wasm";
    let mut cmd = match (wasm, wasm_toolchain()) {
        // The wasm32 standard library belongs to a rustup toolchain, which may
        // not be the `cargo` found first on PATH.
        (true, Some((cargo, rustc))) => {
            let mut cmd = Command::new(cargo);
            // The toolchain's linker (rust-lld) loads libLLVM from the
            // toolchain's lib/ through the library path that rustup's proxies
            // set; the binaries called directly need it set here.
            if let Some(lib) = Path::new(&rustc).parent().and_then(Path::parent).map(|t| t.join("lib")) {
                for var in ["DYLD_FALLBACK_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
                    let mut paths = vec![lib.clone()];
                    paths.extend(std::env::var_os(var).iter().flat_map(std::env::split_paths));
                    if let Ok(joined) = std::env::join_paths(paths) {
                        cmd.env(var, joined);
                    }
                }
            }
            cmd.env("RUSTC", rustc);
            cmd
        }
        _ => Command::new(std::env::var("SCRUST_CARGO").unwrap_or_else(|_| "cargo".into())),
    };
    cmd.arg("build").arg("--quiet").arg("--manifest-path").arg(dir.join("Cargo.toml")).arg("--target-dir").arg(&target_dir);
    if c.release {
        cmd.arg("--release");
    }
    if wasm {
        cmd.arg("--target").arg("wasm32-unknown-unknown");
    }
    let status = cmd.status().map_err(|e| format!("failed to run cargo: {e}"))?;
    if !status.success() {
        return Err("cargo build of the generated crate failed".into());
    }
    let profile = if c.release { "release" } else { "debug" };
    let mut bin = target_dir.clone();
    if wasm {
        bin = bin.join("wasm32-unknown-unknown");
    }
    bin = bin.join(profile).join(if wasm { format!("{crate_name}.wasm") } else { crate_name.to_string() });
    Ok(bin)
}

/// (cargo, rustc) of the active rustup toolchain, if rustup is installed.
fn wasm_toolchain() -> Option<(String, String)> {
    let which = |tool: &str| -> Option<String> {
        let out = Command::new("rustup").args(["which", tool]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if p.is_empty() {
            None
        } else {
            Some(p)
        }
    };
    Some((which("cargo")?, which("rustc")?))
}

fn check_unsupported(c: &Cli, comp: &Compiled) -> Result<(), String> {
    if comp.program.unsupported.is_empty() || c.allow_unsupported {
        return Ok(());
    }
    Err(format!(
        "unsupported blocks (use --allow-unsupported to build anyway; they become no-ops):\n{}",
        unsupported_report(&comp.program).join("\n")
    ))
}

fn runtime_flags(c: &Cli) -> Vec<String> {
    let mut v = Vec::new();
    if c.deterministic {
        v.push("--deterministic".into());
    }
    if let Some(s) = c.seed {
        v.extend(["--seed".into(), s.to_string()]);
    }
    if let Some(f) = c.fps {
        v.extend(["--fps".into(), f.to_string()]);
    }
    if let Some(m) = c.max_passes {
        v.extend(["--max-passes".into(), m.to_string()]);
    }
    if c.turbo {
        v.push("--turbo".into());
    }
    if let Some(a) = &c.answers {
        v.extend(["--answers".into(), a.clone()]);
    }
    v
}

fn types_report(comp: &Compiled) -> String {
    let mut s = String::new();
    let an = &comp.analysis;
    for (ti, t) in comp.program.targets.iter().enumerate() {
        s.push_str(&format!("{}:\n", t.name));
        for (i, v) in t.vars.iter().enumerate() {
            s.push_str(&format!("  var  {:<24} {}\n", v.name, an.explain(Node::Var(VarId { target: ti, index: i }))));
        }
        for (i, l) in t.lists.iter().enumerate() {
            s.push_str(&format!("  list {:<24} {}\n", l.name, an.explain(Node::List(ListId { target: ti, index: i }))));
        }
        for (pi, p) in t.procs.iter().enumerate() {
            let id = scrust_ir::ProcId { target: ti, index: pi };
            let variants: Vec<String> = [false, true]
                .iter()
                .filter(|w| an.variants.contains(&(id, **w)))
                .map(|w| format!("{}:{}", if *w { "warp" } else { "normal" }, if an.yields[&(id, *w)] { "async" } else { "sync" }))
                .collect();
            s.push_str(&format!("  proc {:<24} [{}]\n", p.proccode, variants.join(", ")));
            for (ai, a) in p.params.iter().enumerate() {
                s.push_str(&format!("    arg {:<21} {}\n", a.name, an.explain(Node::Param(id, ai))));
            }
        }
    }
    s
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn real_main() -> Result<ExitCode, String> {
    let c = parse()?;
    match c.cmd.as_str() {
        "check" => {
            let comp = compile(&c)?;
            let scripts: usize = comp.program.targets.iter().map(|t| t.scripts.len()).sum();
            let procs: usize = comp.program.targets.iter().map(|t| t.procs.len()).sum();
            println!("targets: {}  scripts: {}  procedures: {}", comp.program.targets.len(), scripts, procs);
            if !comp.program.extensions.is_empty() {
                println!("extensions: {}", comp.program.extensions.join(", "));
            }
            if comp.program.unsupported.is_empty() {
                println!("unsupported blocks: none");
            } else {
                println!("unsupported blocks:\n{}", unsupported_report(&comp.program).join("\n"));
            }
            if c.types {
                print!("{}", types_report(&comp));
            }
            Ok(if comp.program.unsupported.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(2) })
        }
        "emit" => {
            let comp = compile(&c)?;
            check_unsupported(&c, &comp)?;
            let dir = c.out.clone().ok_or("emit requires -o <dir>")?;
            emit(&c, &comp, &dir, c.headless)?;
            println!("wrote {}", dir.display());
            Ok(ExitCode::SUCCESS)
        }
        "trace" if c.interp => {
            // The interpreter (scrust-interp) instead of a compiled program,
            // with the same runtime flags.
            let input = c.input.as_ref().ok_or("missing input .sb3")?;
            let bytes = std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
            let cfg = scrust_interp::Config { fps: c.fps, turbo: c.turbo, warp_timer: c.warp_timer, headless: true };
            let built = scrust_interp::build(&bytes, &cfg)?;
            if !built.unsupported.is_empty() && !c.allow_unsupported {
                return Err(format!("unsupported blocks (use --allow-unsupported to run anyway; they become no-ops):\n{}", built.unsupported.join("\n")));
            }
            let frames = c.frames.ok_or("trace requires --frames N")?;
            let mut argv = vec!["--deterministic".to_string()];
            argv.extend(runtime_flags(&c));
            argv.extend(["--trace".to_string(), frames.to_string()]);
            scrust_runtime::cli::main_with_args(built.program, built.options, &argv);
            Ok(ExitCode::SUCCESS)
        }
        "build" | "run" | "trace" => {
            let comp = compile(&c)?;
            check_unsupported(&c, &comp)?;
            let headless = c.headless || c.cmd == "trace";
            let stem = c.input.as_ref().and_then(|p| p.file_stem()).map(|s| sanitize(&s.to_string_lossy())).unwrap_or_default();
            let variant = format!(
                "{}{}{}{}",
                stem,
                if headless { "_headless" } else { "" },
                if c.no_unbox { "_nounbox" } else { "" },
                if c.target == "wasm" { "_web" } else { "" }
            );
            let dir = gen_root().join(&variant);
            let crate_name = emit(&c, &comp, &dir, headless)?;
            let bin = cargo_build(&c, &dir, &crate_name)?;
            match c.cmd.as_str() {
                "build" => {
                    let out_dir = c.out.clone().unwrap_or_else(|| PathBuf::from("out"));
                    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
                    let dest = out_dir.join(bin.file_name().unwrap_or_default());
                    std::fs::copy(&bin, &dest).map_err(|e| format!("cannot copy {}: {e}", bin.display()))?;
                    if c.target == "wasm" {
                        let html = crates_dir().join("scrust-player").join("web").join("index.html");
                        let page = std::fs::read_to_string(&html).map_err(|e| format!("cannot read {}: {e}", html.display()))?;
                        let page = if c.single_html {
                            let data = std::fs::read(&dest).map_err(|e| e.to_string())?;
                            let _ = std::fs::remove_file(&dest);
                            page.replace("__WASM_BASE64__", &base64(&data))
                        } else {
                            page.replace("__WASM__", &dest.file_name().unwrap_or_default().to_string_lossy())
                        };
                        std::fs::write(out_dir.join("index.html"), page).map_err(|e| e.to_string())?;
                        println!("built {}", out_dir.join("index.html").display());
                        return Ok(ExitCode::SUCCESS);
                    }
                    if c.external_assets {
                        let adir = out_dir.join("assets");
                        std::fs::create_dir_all(&adir).map_err(|e| e.to_string())?;
                        for (name, data) in &comp.project.assets {
                            std::fs::write(adir.join(name), data).map_err(|e| e.to_string())?;
                        }
                    }
                    println!("built {}", dest.display());
                    Ok(ExitCode::SUCCESS)
                }
                "run" => {
                    let mut cmd = Command::new(&bin);
                    cmd.args(runtime_flags(&c)).args(&c.passthrough);
                    if let Some(f) = c.frames {
                        cmd.args(["--frames", &f.to_string()]);
                    }
                    let st = cmd.status().map_err(|e| e.to_string())?;
                    Ok(if st.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
                }
                _ => {
                    let frames = c.frames.ok_or("trace requires --frames N")?;
                    let mut cmd = Command::new(&bin);
                    cmd.arg("--deterministic").args(runtime_flags(&c)).args(["--trace", &frames.to_string()]);
                    let st = cmd.status().map_err(|e| e.to_string())?;
                    Ok(if st.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
                }
            }
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
