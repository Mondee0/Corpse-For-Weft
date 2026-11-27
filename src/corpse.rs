//! The corpse entity, its menu and what happens when someone dies.

use weft_sdk::*;

/// The body left behind. Its look is the skin PNG shipped in this mod (or replaced by a resource
/// pack), since the public SDK cannot read another player's skin.
pub static CORPSE: EntityKind = EntityKind::named("corpse");
/// A second, skeleton-skinned look for a corpse that has been emptied. It is only a look: the
/// entity itself never changes, so its inventory and equipment stay with it.
static CORPSE_SKELETON: EntityKind = EntityKind::named("skeleton");
static CORPSE_MENU: Menu = Menu::named("corpse");
/// Client button -> server: move everything from the corpse into the clicking player.
static TRANSFER: Channel<u64> = Channel::named("corpse_transfer");
/// Set once the body has become a skeleton, so the check runs only until then.
pub static BONES: Component<u8> = Component::named("corpse:bones");
/// The age at which the last item was taken. Zero while it still carries something.
static EMPTY_AT: Component<u64> = Component::named("corpse:empty_at");
/// The yaw it was left at, so nothing can turn the body over time.
static FACING: Component<f32> = Component::named("corpse:facing");
/// Two minutes after the corpse is emptied it fades away. A body with items never times out.
const AFTER_EMPTY_TICKS: u64 = 2 * 60 * 20;

/// The classic box unwrap, in the engine's face order: -X, +X, -Y, +Y, -Z, +Z.
fn skin_faces(uv: [f32; 2], size: [f32; 3], mirror: bool) -> [[f32; 4]; 6] {
    let [w, h, d] = size;
    let [u, v] = uv;
    let mut faces = [
        [u + d + w, v + d, d, h],
        [u, v + d, d, h],
        [u + d + w, v, w, d],
        [u + d + w, v + d, -w, -d],
        [u + d, v + d, w, h],
        [u + d * 2.0 + w, v + d, w, h],
    ];
    if mirror {
        faces.swap(0, 1);
        for f in &mut faces {
            f[0] += f[2];
            f[2] = -f[2];
        }
    }
    faces
}

/// The player's skin laid out along the ground. Forced poses are only honoured on the local
/// player, so a body has to be built lying down rather than stood up and then tipped over.
fn lying_humanoid() -> Model {
    let standing = humanoid(false);
    let parts = standing
        .parts
        .into_iter()
        .enumerate()
        .map(|(index, mut part)| {
            // A quarter turn about x: the face (-z in model space) ends up pointing up.
            // The +4 pixels lift the body so it rests on the grass instead of half inside it.
            part.pivot = [part.pivot[0], 4.0 - part.pivot[2], part.pivot[1]];
            let head = part.name == "head";
            for box_ in &mut part.boxes {
                let old_faces = skin_faces(box_.uv, box_.extent, box_.mirror != 0);
                box_.origin = [box_.origin[0], -box_.origin[2], box_.origin[1]];
                box_.extent = [box_.extent[0], box_.extent[2], box_.extent[1]];
                if head {
                    // Centred between the shoulders, not sitting on top of or sunk below them.
                    box_.origin[1] -= 1.5;
                    box_.origin[2] -= 2.0;
                }
                // Give the head its own mapping: the face texels go on the upward face. The
                // other parts just follow the rotation of the classic unwrap.
                box_.per_face = 1;
                box_.faces = if head {
                    [[16.0, 8.0, 8.0, 8.0], [0.0, 8.0, 8.0, 8.0], [16.0, 0.0, 8.0, 8.0], [8.0, 16.0, 8.0, -8.0],
                        [24.0, 8.0, 8.0, 8.0], [16.0, 8.0, 8.0, 8.0]]
                } else {
                    [old_faces[0], old_faces[1], old_faces[5], old_faces[4], old_faces[2], old_faces[3]]
                };
            }
            // Inert bone names: no head look, no limb swings, no hair layer added by the engine.
            part.name = format!("corpse{index}");
            part
        })
        .collect();
    Model { parts, uv: standing.uv }
}

pub fn register(r: &mut Registry) {
    r.entity_component(&BONES, 0u8, component_flags::SAVE | component_flags::SYNC_WATCHERS);
    r.entity_component(&EMPTY_AT, 0u64, component_flags::SAVE);
    r.entity_component(&FACING, 0.0f32, component_flags::SAVE);

    r.entity(&CORPSE_SKELETON)
        .no_egg()
        .size(0.6, 1.8)
        .equipment()
        .model(lying_humanoid())
        .texture("skeleton");

    // A player has 36 carried slots (9 hotbar, 27 main) and 5 equipment slots. `.equipment()`
    // appends the four worn slots and the off hand at 36..41, in the same order as a player's,
    // so every stack can be copied by its slot number.
    r.entity(&CORPSE)
        .no_egg()
        .size(0.6, 1.8)
        .eye_height(1.62)
        .passive()
        .invulnerable()
        .kinematic()
        .unpushable()
        .tag("immune/void")
        .tag("immune/lava")
        .tag("immune/fire")
        .attribute(&attribute::BURNING_TIME, 0.0)
        .inventory(36)
        .equipment()
        .model(lying_humanoid())
        .texture("corpse")
        .attach(&BONES)
        .attach(&EMPTY_AT)
        .attach(&FACING)
        .on_interact(|e| e.world().open_menu(e.other(), &CORPSE_MENU, e.entity()))
        .on_tick(turn_to_skeleton);

    r.menu(&CORPSE_MENU)
        .group("items", Slots::chest(3).holder().at(9))
        .group("hotbar", Slots::row(9).holder().at(0))
        .equipment()
        .player_inventory()
        .screen(|ui| {
            ui.panel("panel", S::column().pad(7.0).gap(4.0), |ui| {
                ui.title();
                ui.row(S::row().gap(6.0), |ui| {
                    ui.group("items");
                    ui.column(S::column().gap(2.0), |ui| {
                        ui.equipment();
                        ui.group("hotbar");
                    });
                });
                ui.player_inventory(true);
                ui.button("corpse.transfer", S::default(), |c| {
                    if let Some(owner) = c.menu_owner() {
                        c.client().send(&TRANSFER, &owner);
                    }
                });
            });
        });

    r.on_event(&actions::DIE, on_death);
    r.packet(&TRANSFER, transfer);
}

/// Mod event handlers run before the engine drops a player's inventory, so clearing the slots
/// here leaves nothing to fall on the ground.
fn on_death(b: &EventBatch<abi::Death>) {
    let w = b.world();
    for d in b.iter() {
        if d.kind != PLAYER.id() || settings::KEEP_INVENTORY.get() {
            continue;
        }
        let yaw = w.entity(d.entity).map_or(0.0, |e| e.yaw);
        let corpse = w.spawn_with(&CORPSE, d.pos, &Spawn::new().yaw(yaw));
        if corpse == 0 {
            continue;
        }
        w.entity_modify(corpse).persistent(true);
        if let Some(facing) = w.component_mut(corpse, &FACING) {
            *facing = yaw;
        }
        w.set_pose(corpse, Some(&poses::SLEEP), Some(yaw.to_degrees()));

        let from = w.inventory(d.entity);
        let to = w.inventory(corpse);
        for i in 0..from.len().min(to.len()) {
            let stack = from.get(i);
            if !stack.is_empty() {
                to.set(i, stack);
                from.set(i, ItemStack::EMPTY);
            }
        }

    }
}

fn turn_to_skeleton(t: &EntityTick) {
    let w = t.world();
    for &id in t.ids() {
        // No AI, no gravity and no pushes: hold it exactly where it was left, every tick.
        let facing = w.component(id, &FACING).unwrap_or(0.0);
        let _keep = w.entity_modify(id).velocity([0.0, 0.0, 0.0]).yaw(facing).pitch(0.0).fire(0);
        if w.component(id, &BONES).unwrap_or(0) != 0 {
            // The countdown begins when it was emptied, not when the player died.
            let emptied = w.component(id, &EMPTY_AT).unwrap_or(0);
            if emptied != 0
                && w.entity(id)
                    .is_some_and(|st| st.age.saturating_sub(emptied) >= AFTER_EMPTY_TICKS)
            {
                w.despawn(id);
            }
            continue;
        }
        let inv = w.inventory(id);
        let empty = (0..inv.len()).all(|i| inv.get(i).is_empty());
        if empty {
            w.set_appearance(id, Some(CORPSE_SKELETON.name()));
            if let Some(bones) = w.component_mut(id, &BONES) {
                *bones = 1;
            }
            // +1 so that an empty body found on its first tick is still marked.
            let at = w.entity(id).map_or(1, |st| st.age + 1);
            if let Some(when) = w.component_mut(id, &EMPTY_AT) {
                *when = at;
            }
        }
    }
}

fn transfer(p: &Packet, owner: u64) {
    let w = p.world();
    let Some(st) = w.entity(owner) else { return };
    if st.kind != CORPSE.id() || !w.in_reach(p.player(), owner) {
        return;
    }
    let from = w.inventory(owner);
    let to = w.inventory(p.player());
    for i in 0..from.len() {
        let stack = from.get(i);
        if stack.is_empty() {
            continue;
        }
        let left = to.insert(stack);
        from.set(i, left);
    }
}
