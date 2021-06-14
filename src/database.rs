use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Statement, NO_PARAMS};
use std::convert::{TryFrom, TryInto};

use crate::model::{Assignment, Review, Srs, Stage, Subject, SubjectID};

const DB_PATH: &str = "wanikani_cache.db";

pub fn open() -> rusqlite::Result<Connection> {
    Connection::open(DB_PATH)
}

pub struct DatabaseWrapper<'a> {
    select_reviews_stmt: Statement<'a>,
    select_subjects_stmt: Statement<'a>,
    select_assignments_stmt: Statement<'a>,
    select_next_review_time_stmt: Statement<'a>,
}

impl<'a> DatabaseWrapper<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        let select_reviews_stmt = conn
            .prepare("SELECT json_extract(data, '$.data') FROM reviews")
            .unwrap();

        let select_subjects_stmt = conn
            .prepare("SELECT id, object, json_extract(data, '$.data') FROM subjects")
            .unwrap();

        let select_assignments_stmt = conn
            .prepare("SELECT json_extract(data, '$.data') FROM assignments")
            .unwrap();

        let select_next_review_time_stmt = conn
            .prepare("SELECT min(json_extract(data, '$.data.available_at')) FROM assignments")
            .unwrap();

        Self {
            select_reviews_stmt,
            select_subjects_stmt,
            select_assignments_stmt,
            select_next_review_time_stmt,
        }
    }

    fn json_to_srs(value: &serde_json::Value) -> Srs {
        u8::try_from(value.as_i64().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn json_to_stage(value: &serde_json::Value) -> Stage {
        u8::try_from(value.as_i64().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn json_to_subject_id(value: &serde_json::Value) -> SubjectID {
        SubjectID(value.as_i64().unwrap().try_into().unwrap())
    }

    fn json_to_subject_id_list(value: &serde_json::Value) -> Vec<SubjectID> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(Self::json_to_subject_id)
            .collect()
    }

    pub fn reviews(&mut self) -> impl Iterator<Item = rusqlite::Result<Review>> + '_ {
        self.select_reviews_stmt
            .query_map(NO_PARAMS, |row| {
                // TODO: don't set up a full serde_json::Value, avoid copying
                let json: serde_json::Value = row.get(0)?;
                let srs = Self::json_to_srs(&json["spaced_repetition_system_id"]);
                let start_stage = Self::json_to_stage(&json["starting_srs_stage"]);
                let end_stage = Self::json_to_stage(&json["ending_srs_stage"]);

                Ok(Review {
                    srs,
                    start_stage,
                    end_stage,
                })
            })
            .unwrap()
    }

    pub fn subjects(&mut self) -> impl Iterator<Item = rusqlite::Result<Subject>> + '_ {
        self.select_subjects_stmt
            .query_map(NO_PARAMS, |row| {
                let id = SubjectID(row.get::<_, i64>(0)?.try_into().unwrap());

                let object = row
                    .get_raw_checked(1)?
                    .as_str()
                    .unwrap()
                    .try_into()
                    .unwrap();

                // TODO: don't set up a full serde_json::Value, avoid copying
                let json: serde_json::Value = row.get(2)?;

                let level = json["level"].as_i64().unwrap().try_into().unwrap();

                // let kind = serde_json::from_value()

                let depends_on = json
                    .get("component_subject_ids")
                    .map_or(vec![], Self::json_to_subject_id_list);
                let depended_on_by = json
                    .get("amalgamation_subject_ids")
                    .map_or(vec![], Self::json_to_subject_id_list);

                let srs = Self::json_to_srs(&json["spaced_repetition_system_id"]);

                Ok({
                    Subject {
                        id,
                        level,
                        kind: object,
                        depends_on,
                        depended_on_by,
                        srs,
                    }
                })
            })
            .unwrap()
    }

    pub fn assignments(&mut self) -> impl Iterator<Item = rusqlite::Result<Assignment>> + '_ {
        self.select_assignments_stmt
            .query_map(NO_PARAMS, |row| {
                // TODO: don't set up a full serde_json::Value, avoid copying
                let json: serde_json::Value = row.get(0)?;
                let subject_id = Self::json_to_subject_id(&json["subject_id"]);
                let stage = Self::json_to_stage(&json["srs_stage"]);

                let next_review_time = &json["available_at"];
                let next_review_time = if next_review_time.is_null() {
                    None
                } else {
                    let next_review_time = next_review_time.as_str().unwrap();
                    Some(
                        DateTime::parse_from_rfc3339(next_review_time)
                            .unwrap()
                            .into(),
                    )
                };

                Ok(Assignment {
                    subject_id,
                    stage,
                    next_review_time,
                })
            })
            .unwrap()
    }

    pub fn next_review_time(&mut self) -> Option<DateTime<Local>> {
        self.select_next_review_time_stmt
            .query_row(NO_PARAMS, |row| {
                let time = row.get_raw_checked(0)?.as_str().unwrap();
                Ok(DateTime::parse_from_rfc3339(time).unwrap().into())
            })
            .optional()
            .unwrap()
    }
}
