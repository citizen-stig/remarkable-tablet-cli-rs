//! `SceneTreeBlock` (0x01) and `TreeNodeBlock` (0x02) — spec §5.2 and §5.3.

use crate::crdt::{CrdtId, LwwValue};

#[derive(Debug, Clone)]
pub struct SceneTreeBlock {
    pub tree_id: CrdtId,
    pub node_id: CrdtId,
    pub is_update: bool,
    pub parent_id: CrdtId,
}

#[derive(Debug, Clone)]
pub struct TreeNodeBlock {
    pub node_id: CrdtId,
    pub label: LwwValue<String>,
    pub visible: LwwValue<bool>,
    pub anchor: Option<Anchor>,
}

/// Group anchor — spec §5.3 and §10.2. The two layouts (older indices 4-6
/// without LWW vs. newer 7-10 with LWW) need confirmation against a real
/// anchored-group fixture before the older variant is wired up.
#[derive(Debug, Clone)]
pub struct Anchor {
    pub anchor_id: LwwValue<CrdtId>,
    pub anchor_type: LwwValue<u8>,
    pub anchor_threshold: LwwValue<f32>,
    pub anchor_origin_x: LwwValue<f32>,
}
