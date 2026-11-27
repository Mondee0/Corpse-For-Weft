//! A corpse left where a player died, holding what they carried.
//!
//! The body wears a humanoid skin (the PNG in `assets/textures/entity/corpse.png`, replaceable
//! with a resource pack) and lies on the ground. Right click it to open its inventory; a button
//! moves everything into the player's inventory. Once nothing is left, the body turns into a
//! skeleton. Built on the public mod SDK, with no engine changes.

use weft_sdk::*;

declare_mod!(deps: ["base"], init: init);

mod corpse;

fn init(r: &mut Registry) {
    corpse::register(r);
}
