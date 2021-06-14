mod database;
mod model;

use chrono::{Duration, DurationRound};
use indicatif::ProgressBar;
use model::{SubjectKind, MAX_LEVEL};
use rand::{thread_rng, Rng};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    convert::{TryFrom, TryInto},
};

use self::{
    database::DatabaseWrapper,
    model::{Stage, Subject, SubjectID, NUM_STAGES},
};

#[derive(Clone, Debug)]
struct StageProbabilityDistribution {
    /// Stage probabilities should all be divided by this value
    total: u32,
    stage_probs: Vec<(Stage, u32)>,
}

impl StageProbabilityDistribution {
    pub fn new(stage_probs: Vec<(Stage, u32)>) -> Self {
        let total = stage_probs.iter().map(|(_stage, n)| n).sum();

        Self { total, stage_probs }
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn sample(&self) -> Option<Stage> {
        let mut x = thread_rng().gen_range(0..self.total);

        for (stage, stage_n) in &self.stage_probs {
            if x < *stage_n {
                return Some(*stage);
            } else {
                x -= stage_n;
            }
        }

        None
    }

    /// Create a new distribution based on this one, but shifted up by `x`
    pub fn shift(&self, x: isize) -> Self {
        let mut new_dist = self.clone();
        for (stage, _) in &mut new_dist.stage_probs {
            let new_stage = isize::from(u8::from(*stage)) + x;
            let new_stage = new_stage.clamp(0, isize::try_from(NUM_STAGES - 1).unwrap());
            let new_stage = u8::try_from(new_stage).unwrap();
            *stage = Stage::try_from(new_stage).unwrap();
        }
        new_dist
    }
}

#[derive(Debug)]
struct ReviewResultProbability {
    by_prev_stage: [StageProbabilityDistribution; NUM_STAGES],
}

impl ReviewResultProbability {
    pub fn new(db: &mut DatabaseWrapper) -> Self {
        let mut stage_counts = [[0; NUM_STAGES]; NUM_STAGES];
        for review in db.reviews() {
            let review = review.unwrap();
            stage_counts[review.start_stage as usize][review.end_stage as usize] += 1;
        }

        let mut by_prev_stage: [StageProbabilityDistribution; NUM_STAGES] = stage_counts
            .iter()
            .map(|end_stage_counts| {
                StageProbabilityDistribution::new(
                    end_stage_counts
                        .iter()
                        .copied()
                        .enumerate()
                        .filter_map(|(end_stage, count)| {
                            if count > 0 {
                                let end_stage = u8::try_from(end_stage).unwrap();
                                let end_stage = Stage::try_from(end_stage).unwrap();
                                Some((end_stage, count))
                            } else {
                                None
                            }
                        })
                        .collect(),
                )
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        // Fill in unknown probabilities for higher stages (except "burned"),
        // with those of the rows preceding them.
        let last_non_empty_row = by_prev_stage
            .iter()
            .rposition(|row| !row.is_empty())
            // TODO: don't panic here
            .expect("No reviews at all?");
        let (until_last, empty_rows) = by_prev_stage.split_at_mut(last_non_empty_row + 1);
        let last_non_empty_row = until_last.last().unwrap();
        let (_burned_row, empty_rows) = empty_rows.split_last_mut().unwrap();
        for (i, row) in empty_rows.iter_mut().enumerate() {
            // TODO: instead of shifting, store the "correct results"
            // probabilities, then implement the stage distribution based on
            // that + the formula from the wk knowledge guide
            *row = last_non_empty_row.shift(isize::try_from(i + 1).unwrap());
        }

        Self { by_prev_stage }
    }

    pub fn sample_for(&self, prev_stage: Stage) -> Option<Stage> {
        self.by_prev_stage[prev_stage as usize].sample()
    }
}

fn load_subjects(db: &mut DatabaseWrapper) -> HashMap<SubjectID, Subject> {
    db.subjects()
        .map(|subject| {
            let subject = subject.unwrap();
            (subject.id, subject)
        })
        .collect()
}

fn subjects_with_level(subjects: &HashMap<SubjectID, Subject>, level: u8) -> Vec<SubjectID> {
    subjects
        .iter()
        .filter_map(|(subject_id, subject)| {
            if subject.level == level {
                Some(*subject_id)
            } else {
                None
            }
        })
        .collect()
}

#[derive(Clone)]
struct SubjectState {
    stage: Stage,
    /// Simulation step number at which the subject can be reviewed again. None
    /// means that it's burned!
    next_review_time: Option<u32>,
}

impl SubjectState {
    pub fn newly_unlocked(cur_time: u32) -> Self {
        Self {
            stage: Stage::Apprentice1,
            next_review_time: Some(cur_time),
        }
    }
}

#[derive(Clone)]
struct Simulator<'a> {
    review_prob: &'a ReviewResultProbability,
    subjects: &'a HashMap<SubjectID, Subject>,
    cur_step: u32,
    subject_states: HashMap<SubjectID, SubjectState>,
    review_queue: BinaryHeap<(Reverse<u32>, SubjectID)>,
    cur_level: u8,
    cur_level_subjects: Vec<SubjectID>,
    cur_level_kanji: Vec<SubjectID>,
}

impl<'a> Simulator<'a> {
    pub fn new(
        review_prob: &'a ReviewResultProbability,
        subjects: &'a HashMap<SubjectID, Subject>,
        db: &mut DatabaseWrapper,
    ) -> Self {
        let base_time = db
            .next_review_time()
            .expect("No available reviews")
            // Round down to hour
            .duration_trunc(Duration::hours(1))
            .unwrap();

        let subject_states = db
            .assignments()
            .map(|assignment| {
                let assignment = assignment.unwrap();

                let mut stage = assignment.stage;

                let steps_from_base = if let Some(next_review_time) = assignment.next_review_time {
                    assert_ne!(stage, Stage::Initiate);
                    let time_since = next_review_time.signed_duration_since(base_time);
                    time_since.num_hours().max(0).try_into().unwrap()
                } else {
                    assert_eq!(stage, Stage::Initiate);
                    // Pretend we did the lesson, skip initiate stage.
                    stage = Stage::Apprentice1;
                    0
                };

                (
                    assignment.subject_id,
                    SubjectState {
                        stage,
                        next_review_time: Some(steps_from_base),
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        let review_queue = subject_states
            .iter()
            .filter_map(|(subject_id, state)| Some((Reverse(state.next_review_time?), *subject_id)))
            .collect();

        // The current level is the highest level for an unlocked subject, and
        // unlocked subjects are those included in subject_states.
        let cur_level = subject_states
            .keys()
            .map(|subject_id| subjects[subject_id].level)
            .max()
            .expect("No unlocked subjects");

        let cur_level_subjects = subjects_with_level(subjects, cur_level);
        let cur_level_kanji = cur_level_subjects
            .iter()
            .copied()
            .filter(|subject_id| subjects[subject_id].kind == SubjectKind::Kanji)
            .collect();

        Self {
            review_prob,
            subjects,
            cur_step: 0,
            subject_states,
            review_queue,
            cur_level,
            cur_level_subjects,
            cur_level_kanji,
        }
    }

    fn peek_available_review(&self) -> Option<SubjectID> {
        let (Reverse(next_review_time), subject_id) = self.review_queue.peek()?;

        if *next_review_time <= self.cur_step {
            Some(*subject_id)
        } else {
            None
        }
    }

    fn pop_available_review(&mut self) -> Option<SubjectID> {
        if let Some(subject_id) = self.peek_available_review() {
            self.review_queue.pop().unwrap();
            Some(subject_id)
        } else {
            None
        }
    }

    /// Returns number of reviews performed in this step
    fn step(&mut self) -> u32 {
        let mut review_count = 0;

        // Loop until done unlocking levels
        while self.peek_available_review().is_some() {
            // Loop over subjects up to current level
            while let Some(subject_id) = self.pop_available_review() {
                let subject = &self.subjects[&subject_id];
                let subject_state = self.subject_states.get_mut(&subject_id).unwrap();

                review_count += 1;
                let old_stage = subject_state.stage;
                let new_stage = self.review_prob.sample_for(old_stage).unwrap();

                subject_state.stage = new_stage;
                if let Some(hours_to_next_review) = subject.srs.hours_to_next_review(new_stage) {
                    // Reschedule
                    let next_review_time = self.cur_step + hours_to_next_review;
                    subject_state.next_review_time = Some(next_review_time);
                    self.review_queue
                        .push((Reverse(next_review_time), subject_id));
                } else {
                    // Burned!
                    debug_assert_eq!(new_stage, Stage::Burned);
                    subject_state.next_review_time = None;
                    // No need to reschedule in review_queue
                }

                if !old_stage.is_passing() && new_stage.is_passing() {
                    // Check if we unlocked stuff
                    for subject2_id in &subject.depended_on_by {
                        let subject2_id = *subject2_id;

                        // Ignore if already unlocked
                        if self.subject_states.contains_key(&subject2_id) {
                            continue;
                        }

                        if self.may_unlock(subject2_id) {
                            self.subject_states
                                .insert(subject2_id, SubjectState::newly_unlocked(self.cur_step));
                            // Prepare to do review immediately
                            self.review_queue
                                .push((Reverse(self.cur_step), subject2_id));
                        }
                    }
                }
            }

            // Check if done with current level
            if self.cur_level < MAX_LEVEL && self.passed_current_level() {
                self.cur_level += 1;
                self.cur_level_subjects = subjects_with_level(self.subjects, self.cur_level);
                self.cur_level_kanji = self
                    .cur_level_subjects
                    .iter()
                    .copied()
                    .filter(|subject_id| self.subjects[subject_id].kind == SubjectKind::Kanji)
                    .collect();

                // Check if we unlocked stuff
                for subject_id in &self.cur_level_subjects {
                    let subject_id = *subject_id;

                    if self.may_unlock(subject_id) {
                        self.subject_states
                            .insert(subject_id, SubjectState::newly_unlocked(self.cur_step));

                        // Prepare to do review immediately
                        self.review_queue.push((Reverse(self.cur_step), subject_id));
                    }
                }
            }
        }

        self.cur_step += 1;

        review_count
    }

    fn passed_current_level(&self) -> bool {
        let num_kanji = self.cur_level_kanji.len();
        let mut num_passed_kanji = 0;
        for subject_id in &self.cur_level_kanji {
            if let Some(subject_state) = self.subject_states.get(subject_id) {
                if subject_state.stage.is_passing() {
                    num_passed_kanji += 1;
                }
            }
        }

        num_passed_kanji >= (num_kanji * 9) / 10
    }

    fn may_unlock(&self, subject_id: SubjectID) -> bool {
        let subject = &self.subjects[&subject_id];

        // Must be at least current level to unlock
        if subject.level > self.cur_level {
            return false;
        }

        // Check if we unlocked all requirements
        if !subject
            .depends_on
            .iter()
            .all(|subject2_id| self.subject_states.contains_key(subject2_id))
        {
            return false;
        }

        true
    }
}

fn main() {
    let db = database::open().unwrap();
    let mut db = DatabaseWrapper::new(&db);

    let review_prob = ReviewResultProbability::new(&mut db);
    let subjects = load_subjects(&mut db);

    let sim = Simulator::new(&review_prob, &subjects, &mut db);

    let num_runs = 1000;
    let mut day_counts = [0; 365];
    let mut levels = [0; 365];
    let pb = ProgressBar::new(num_runs);
    for _run in 0..num_runs {
        pb.inc(1);

        let mut sim = sim.clone();

        for (day_count, level) in day_counts.iter_mut().zip(levels.iter_mut()) {
            // Cast to u32 so that it's big enough to store the sum
            *level += u32::from(sim.cur_level);
            *day_count += (0..24).map(|_| sim.step()).sum::<u32>();
        }
    }

    pb.finish_with_message("done");

    for (day, (day_count_sum, level_sum)) in day_counts
        .iter()
        .copied()
        .zip(levels.iter().copied())
        .enumerate()
    {
        let avg_day_count = day_count_sum as f32 / num_runs as f32;
        let avg_level = level_sum as f32 / num_runs as f32;
        println!(
            "Day {:>3}: level {:>2}, {:>4} reviews",
            day,
            avg_level.round() as u32,
            avg_day_count.round() as u32
        );
    }
}
