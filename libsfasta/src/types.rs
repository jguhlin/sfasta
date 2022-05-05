#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct Location {
    pub sequence: Option<Vec<Loc>>,
    pub masking: Option<Vec<Loc>>,
    pub scores: Option<Vec<Loc>>,
    pub seqinfo: Option<Vec<Loc>>,
}

impl Location {
    pub fn new() -> Location {
        Location {
            sequence: None,
            masking: None,
            scores: None,
            seqinfo: None,
        }
    }
}

pub type Loc = (u32, (u32, u32));
