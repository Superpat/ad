//! vim style insert mode where most keys are directly modifying the buffer
use crate::{
    editor::{Action::*, Actions},
    key::{Arrow::*, Key::*},
    keymap,
    mode::Mode,
    term::CurShape,
};

pub(crate) fn insert_mode() -> Mode {
    let mut keymap = keymap! {
        [ Esc ] => [ SetMode { m: "NORMAL" } ],
        [ Char('f'), Char('d') ] => [ SetMode { m: "NORMAL" } ],
        [ Backspace ] => [ DeleteChar ],
        [ Del ] => [ Move { d: Right, n: 1 }, DeleteChar ]

    };

    // By default we just let the buffer try to handle this
    keymap.set_default(|&k| Some(Actions::Single(RawKey { k })));

    Mode {
        name: "INSERT".to_string(),
        cur_shape: CurShape::Bar,
        keymap,
        handle_expired_pending: |keys| {
            Some(if keys.len() == 1 {
                Actions::Single(RawKey { k: keys[0] })
            } else {
                Actions::Multi(keys.iter().map(|&k| RawKey { k }).collect())
            })
        },
    }
}
