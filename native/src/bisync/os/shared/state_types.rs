use super::types::Sig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::A => "A",
            Side::B => "B",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PairRecord {
    pub pair: String,
    pub root_a: String,
    pub root_b: String,
    pub mode: String,
    pub source_side: Side,
    pub source_cursor: Option<String>,
    pub root_a_id: Option<String>,
    pub root_b_id: Option<String>,
    pub bootstrapped: bool,
    pub target_managed: bool,
}

#[derive(Clone, Debug)]
pub struct ItemRecord {
    pub side: Side,
    pub rel: String,
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub sig: Option<Sig>,
    pub is_dir: bool,
    pub deleted: bool,
}
