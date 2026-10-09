//! Keyboard handling shared by the native and web hosts.

use scrust_render::SoftRenderer;
use scrust_runtime::{JsStr, World};

fn renderer(world: &mut World) -> Option<&mut SoftRenderer> {
    world.renderer.as_mut()?.as_any_mut().downcast_mut::<SoftRenderer>()
}

/// Handle a key event. `key` is a DOM `KeyboardEvent.key` value; `text` is
/// the text the key produced, if any. While a question is open, typing goes
/// to the prompt instead of the project.
pub fn key_event(world: &mut World, key: &str, text: Option<&str>, down: bool) {
    if world.current_question().is_some() {
        if !down {
            return;
        }
        match key {
            "Enter" => {
                let answer = renderer(world).map(|r| std::mem::take(&mut r.ask_input)).unwrap_or_default();
                world.submit_answer(JsStr::from(answer.as_str()));
            }
            "Backspace" => {
                if let Some(r) = renderer(world) {
                    r.ask_input.pop();
                }
            }
            _ => {
                if let (Some(t), Some(r)) = (text, renderer(world)) {
                    r.ask_input.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
        }
        return;
    }
    match key {
        "Escape" if down => world.stop_all(),
        "F5" if down => world.green_flag(),
        _ => world.post_key(key, down),
    }
}
