use std::convert::TryFrom;

use chrono::{DateTime, Local};
use num_enum::{IntoPrimitive, TryFromPrimitive};

pub const MAX_LEVEL: u8 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, TryFromPrimitive)]
#[repr(u8)]
pub enum Srs {
    Normal = 1,
    Accelerated,
}

impl Srs {
    pub fn hours_to_next_review(self, stage: Stage) -> Option<u32> {
        use Srs::*;
        use Stage::*;

        Some(match (self, stage) {
            (_, Initiate) | (_, Burned) => {
                return None;
            }

            (Normal, Apprentice1) => 4,
            (Normal, Apprentice2) => 8,
            (Normal, Apprentice3) => 23,
            (Normal, Apprentice4) => 47,

            (Accelerated, Apprentice1) => 2,
            (Accelerated, Apprentice2) => 4,
            (Accelerated, Apprentice3) => 8,
            (Accelerated, Apprentice4) => 23,

            (_, Guru1) => 167,
            (_, Guru2) => 335,
            (_, Master) => 719,
            (_, Enlightened) => 2879,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive, IntoPrimitive)]
#[repr(u8)]
pub enum Stage {
    Initiate = 0,
    Apprentice1 = 1,
    Apprentice2 = 2,
    Apprentice3 = 3,
    Apprentice4 = 4,
    Guru1 = 5,
    Guru2 = 6,
    Master = 7,
    Enlightened = 8,
    Burned = 9,
}

impl Stage {
    pub fn is_passing(self) -> bool {
        self >= Stage::Guru1
    }
}

pub const NUM_STAGES: usize = 10;

#[derive(Debug)]
pub struct Review {
    pub srs: Srs,
    pub start_stage: Stage,
    pub end_stage: Stage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubjectID(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubjectKind {
    Radical,
    Kanji,
    Vocabulary,
}

impl TryFrom<&str> for SubjectKind {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "radical" => Ok(Self::Radical),
            "kanji" => Ok(Self::Kanji),
            "vocabulary" => Ok(Self::Vocabulary),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Subject {
    pub id: SubjectID,
    pub level: u8,
    pub kind: SubjectKind,
    pub depends_on: Vec<SubjectID>,
    pub depended_on_by: Vec<SubjectID>,
    pub srs: Srs,
}

#[derive(Debug)]
pub struct Assignment {
    pub subject_id: SubjectID,
    pub stage: Stage,
    pub next_review_time: DateTime<Local>,
}
