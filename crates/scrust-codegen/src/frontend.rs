//! The front end shared by the compiler (`scrust-cli`) and the interpreter
//! (`scrust-interp`): sb3 -> project -> IR -> analysis.

use scrust_analysis::{Analysis, Ty};
use scrust_ir::{ListId, Program, VarId};
use scrust_sb3::Project;

pub struct Compiled {
    pub project: Project,
    pub program: Program,
    pub analysis: Analysis,
}

/// Load, lower and analyse a project.
pub fn compile(bytes: &[u8], no_unbox: bool, warp_timer: bool) -> Result<Compiled, String> {
    let project = Project::load(bytes).map_err(|e| e.to_string())?;
    let program = scrust_ir::lower(&project, &Default::default());
    let mut opts = scrust_analysis::Options { no_unbox, warp_timer, ..Default::default() };
    for m in &project.monitors {
        let sprite = match &m.sprite_name {
            Some(n) => program.targets.iter().position(|t| !t.is_stage && t.name == *n),
            None => Some(0),
        };
        let Some(ti) = sprite else { continue };
        let t = &program.targets[ti];
        match m.opcode.as_str() {
            "data_variable" => {
                if let Some(i) = t.vars.iter().position(|v| v.id == m.id) {
                    opts.monitored_vars.push(VarId { target: ti, index: i });
                }
            }
            "data_listcontents" => {
                if let Some(i) = t.lists.iter().position(|v| v.id == m.id) {
                    opts.monitored_lists.push(ListId { target: ti, index: i });
                }
            }
            _ => {}
        }
    }
    let canonical = |name: &str| Ty::of_value(&scrust_value::Value::str(name)).within(Ty::STR_CANON);
    opts.numeric_asset_names =
        project.targets.iter().any(|t| t.costumes.iter().any(|x| canonical(&x.name)) || t.sounds.iter().any(|x| canonical(&x.name)));
    let analysis = Analysis::run(&program, opts);
    Ok(Compiled { project, program, analysis })
}

/// One line per unsupported opcode and target: `  opcode  (in "Sprite", Nx)`.
pub fn unsupported_report(p: &Program) -> Vec<String> {
    let mut counts: std::collections::BTreeMap<(String, String), usize> = Default::default();
    for (t, op) in &p.unsupported {
        *counts.entry((t.clone(), op.clone())).or_default() += 1;
    }
    counts.into_iter().map(|((t, op), n)| format!("  {op}  (in {t:?}, {n}x)")).collect()
}

/// Frames per second from TurboWarp's project config (`setFramerate`: capped
/// at 250, negative is 1, 0 means the display rate), 30 without one.
pub fn tw_framerate(p: &Project) -> f64 {
    match p.tw_config().framerate {
        None => 30.0,
        Some(f) if f > 250.0 => 250.0,
        Some(f) if f < 0.0 => 1.0,
        Some(f) if f == 0.0 => 60.0,
        Some(f) => f,
    }
}
