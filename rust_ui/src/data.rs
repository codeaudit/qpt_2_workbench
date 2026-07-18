//! Port of the server-relevant part of qpt-data.js: `QPT_DATA.boards`.
//! The JSON was generated mechanically from the original file
//! (`JSON.stringify(QPT_DATA.boards)` → assets/boards.json). `reference` and
//! `learn` are browser-only content and stay in public/qpt-data.js.

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub struct Column {
    pub id: String,
    #[serde(default)]
    pub glyph: Option<String>,
    #[serde(default)]
    pub cat: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub axioms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Board {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "ref")]
    pub ref_: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub columns: Vec<Column>,
    /// seed card literals (heterogeneous per board — kept as JSON values,
    /// exactly as the JS core treats them)
    #[serde(default)]
    pub cards: Vec<Value>,
}

const BOARDS_JSON: &str = include_str!("../assets/boards.json");

pub fn boards() -> &'static [Board] {
    use std::sync::OnceLock;
    static BOARDS: OnceLock<Vec<Board>> = OnceLock::new();
    BOARDS.get_or_init(|| serde_json::from_str(BOARDS_JSON).expect("assets/boards.json is valid"))
}
