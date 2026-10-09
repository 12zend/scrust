//! Table-driven block definitions: simple commands and reporters that map
//! 1:1 onto a runtime method.

/// How an argument is passed to the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    /// `Cast.toNumber`
    Num,
    /// `Cast.toString`
    Str,
    /// `Cast.toBoolean`
    Bool,
    /// Raw value; the string/number distinction is *not* observable.
    Raw,
    /// Raw value reaching a kind-sensitive sink (§8.6).
    RawKind,
}

/// Result type of a reporter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R {
    Num,
    Str,
    Bool,
    Val,
}

#[derive(Clone, Copy, Debug)]
pub enum A {
    /// Block input.
    In(&'static str, K),
    /// Block field (static string).
    Field(&'static str),
}

macro_rules! table {
    ($(#[$m:meta])* $name:ident { $($var:ident = $op:literal, $method:literal, [$($arg:expr),*] $(=> $r:expr)?;)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($var),* }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),*];
            pub fn opcode(self) -> &'static str { match self { $($name::$var => $op),* } }
            pub fn method(self) -> &'static str { match self { $($name::$var => $method),* } }
            pub fn args(self) -> &'static [A] { match self { $($name::$var => &[$($arg),*]),* } }
            pub fn from_opcode(op: &str) -> Option<$name> { match op { $($op => Some($name::$var),)* _ => None } }
        }
    };
}

use A::{Field as F, In as I};
use K::*;

table! {
    /// Non-yielding command blocks implemented by a `World` method.
    Cmd {
        MoveSteps = "motion_movesteps", "move_steps", [I("STEPS", Num)];
        TurnRight = "motion_turnright", "turn_right", [I("DEGREES", Num)];
        TurnLeft = "motion_turnleft", "turn_left", [I("DEGREES", Num)];
        GoTo = "motion_goto", "go_to", [I("TO", Raw)];
        GoToXY = "motion_gotoxy", "go_to_xy", [I("X", Num), I("Y", Num)];
        PointInDirection = "motion_pointindirection", "point_in_direction", [I("DIRECTION", Num)];
        PointTowards = "motion_pointtowards", "point_towards", [I("TOWARDS", Raw)];
        ChangeX = "motion_changexby", "change_x", [I("DX", Num)];
        SetX = "motion_setx", "set_x", [I("X", Num)];
        ChangeY = "motion_changeyby", "change_y", [I("DY", Num)];
        SetY = "motion_sety", "set_y", [I("Y", Num)];
        IfOnEdgeBounce = "motion_ifonedgebounce", "if_on_edge_bounce", [];
        SetRotationStyle = "motion_setrotationstyle", "set_rotation_style", [F("STYLE")];
        Say = "looks_say", "say", [I("MESSAGE", Raw)];
        Think = "looks_think", "think", [I("MESSAGE", Raw)];
        Show = "looks_show", "show", [];
        Hide = "looks_hide", "hide", [];
        SwitchCostume = "looks_switchcostumeto", "switch_costume", [I("COSTUME", RawKind)];
        NextCostume = "looks_nextcostume", "next_costume", [];
        SwitchBackdrop = "looks_switchbackdropto", "switch_backdrop", [I("BACKDROP", RawKind)];
        NextBackdrop = "looks_nextbackdrop", "next_backdrop", [];
        ChangeEffect = "looks_changeeffectby", "change_effect", [F("EFFECT"), I("CHANGE", Num)];
        SetEffect = "looks_seteffectto", "set_effect", [F("EFFECT"), I("VALUE", Num)];
        ClearEffects = "looks_cleargraphiceffects", "clear_effects", [];
        ChangeSize = "looks_changesizeby", "change_size", [I("CHANGE", Num)];
        SetSize = "looks_setsizeto", "set_size", [I("SIZE", Num)];
        GoToFrontBack = "looks_gotofrontback", "go_to_front_back", [F("FRONT_BACK")];
        GoLayers = "looks_goforwardbackwardlayers", "go_layers", [F("FORWARD_BACKWARD"), I("NUM", Num)];
        HideAllSprites = "looks_hideallsprites", "noop0", [];
        ChangeStretch = "looks_changestretchby", "noop1", [I("CHANGE", Raw)];
        SetStretch = "looks_setstretchto", "noop1", [I("STRETCH", Raw)];
        SoundPlay = "sound_play", "sound_play", [I("SOUND_MENU", RawKind)];
        SoundStopAll = "sound_stopallsounds", "sound_stop_all", [];
        SoundChangeEffect = "sound_changeeffectby", "sound_change_effect", [F("EFFECT"), I("VALUE", Num)];
        SoundSetEffect = "sound_seteffectto", "sound_set_effect", [F("EFFECT"), I("VALUE", Num)];
        SoundClearEffects = "sound_cleareffects", "sound_clear_effects", [];
        SoundChangeVolume = "sound_changevolumeby", "sound_change_volume", [I("VOLUME", Num)];
        SoundSetVolume = "sound_setvolumeto", "sound_set_volume", [I("VOLUME", Num)];
        CreateClone = "control_create_clone_of", "create_clone", [I("CLONE_OPTION", Str)];
        ClearCounter = "control_clear_counter", "clear_counter", [];
        IncrCounter = "control_incr_counter", "incr_counter", [];
        SetDragMode = "sensing_setdragmode", "set_drag_mode", [F("DRAG_MODE")];
        ResetTimer = "sensing_resettimer", "reset_timer", [];
        PenClear = "pen_clear", "pen_clear", [];
        PenStamp = "pen_stamp", "pen_stamp", [];
        PenDown = "pen_penDown", "pen_down", [];
        PenUp = "pen_penUp", "pen_up", [];
        PenSetColor = "pen_setPenColorToColor", "pen_set_color", [I("COLOR", Raw)];
        PenChangeParam = "pen_changePenColorParamBy", "pen_change_param", [I("COLOR_PARAM", Str), I("VALUE", Num)];
        PenSetParam = "pen_setPenColorParamTo", "pen_set_param", [I("COLOR_PARAM", Str), I("VALUE", Num)];
        PenChangeSize = "pen_changePenSizeBy", "pen_change_size", [I("SIZE", Num)];
        PenSetSize = "pen_setPenSizeTo", "pen_set_size", [I("SIZE", Num)];
        PenSetShade = "pen_setPenShadeToNumber", "pen_set_shade", [I("SHADE", Num)];
        PenChangeShade = "pen_changePenShadeBy", "pen_change_shade", [I("SHADE", Num)];
        PenSetHue = "pen_setPenHueToNumber", "pen_set_hue", [I("HUE", Num)];
        PenChangeHue = "pen_changePenHueBy", "pen_change_hue", [I("HUE", Num)];
    }
}

impl Cmd {
    /// Commands that ask the renderer to redraw (only when a renderer is attached).
    pub fn is_noop(self) -> bool {
        matches!(self, Cmd::HideAllSprites | Cmd::ChangeStretch | Cmd::SetStretch)
    }
}

table! {
    /// Reporter blocks implemented by a `World` method.
    Rep {
        XPosition = "motion_xposition", "x_position", [];
        YPosition = "motion_yposition", "y_position", [];
        Direction = "motion_direction", "direction", [];
        Size = "looks_size", "size", [];
        Volume = "sound_volume", "volume", [];
        TouchingObject = "sensing_touchingobject", "touching_object", [I("TOUCHINGOBJECTMENU", Raw)];
        TouchingColor = "sensing_touchingcolor", "touching_color", [I("COLOR", Raw)];
        ColorTouchingColor = "sensing_coloristouchingcolor", "color_touching_color", [I("COLOR", Raw), I("COLOR2", Raw)];
        DistanceTo = "sensing_distanceto", "distance_to", [I("DISTANCETOMENU", Raw)];
        Answer = "sensing_answer", "answer", [];
        KeyPressed = "sensing_keypressed", "key_pressed", [I("KEY_OPTION", Raw)];
        MouseDown = "sensing_mousedown", "mouse_down", [];
        MouseX = "sensing_mousex", "mouse_x", [];
        MouseY = "sensing_mousey", "mouse_y", [];
        Loudness = "sensing_loudness", "loudness", [];
        Loud = "sensing_loud", "loud", [];
        Timer = "sensing_timer", "timer", [];
        Current = "sensing_current", "current", [F("CURRENTMENU")];
        DaysSince2000 = "sensing_dayssince2000", "days_since_2000", [];
        Username = "sensing_username", "username", [];
        Counter = "control_get_counter", "counter", [];
        // Split from looks_costumenumbername / looks_backdropnumbername by field.
        CostumeNumber = "looks_costumenumbername#number", "costume_number", [];
        CostumeName = "looks_costumenumbername#name", "costume_name", [];
        BackdropNumber = "looks_backdropnumbername#number", "backdrop_number", [];
        BackdropName = "looks_backdropnumbername#name", "backdrop_name", [];
        // sensing_of with a dynamic object or non-variable property.
        AttrOf = "sensing_of", "attr_of", [F("PROPERTY"), I("OBJECT", Raw)];
    }
}

impl Rep {
    pub fn result(self) -> R {
        use Rep::*;
        match self {
            XPosition | YPosition | Direction | Size | Volume | DistanceTo | MouseX | MouseY | Loudness | Timer | Current
            | DaysSince2000 | Counter | CostumeNumber | BackdropNumber => R::Num,
            TouchingObject | TouchingColor | ColorTouchingColor | KeyPressed | MouseDown | Loud => R::Bool,
            Answer | Username | CostumeName | BackdropName => R::Str,
            AttrOf => R::Val,
        }
    }
    /// Does evaluating this reporter read the per-target state (`me`)?
    pub fn needs_target(self) -> bool {
        true
    }
}
