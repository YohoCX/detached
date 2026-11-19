//! Stats from the owner's dispositions, per role, lens, model, mode and verifier: the share fixed,
//! the share dismissed and why, verify's false confirms and tokens per fixed finding; and the one
//! change they suggest.

use std::collections::{BTreeMap, BTreeSet};

use super::mode::Stage;
use super::record::{self, State};
use super::track::{self, Fate};

/// Fewer findings than this suggest nothing.
const MIN: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum By {
    Role,
    Lens,
    Model,
    Mode,
    Verifier,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Issues raised, once however many rounds raised them, and findings set aside; a verifier's
    /// are the findings it judged.
    pub raised: usize,
    pub fixed: usize,
    pub dismissed: usize,
    pub deferred: usize,
    /// Set aside by a verdict or the quote check.
    pub aside: usize,
    /// Confirmed or refined by verify, and of those, dismissed by the owner.
    pub confirmed: usize,
    pub false_confirms: usize,
    pub tokens: u64,
    pub reasons: BTreeMap<String, usize>,
}

impl Row {
    pub fn open(&self) -> usize {
        let decided = self.fixed + self.dismissed + self.deferred + self.aside;
        self.raised.saturating_sub(decided)
    }

    pub fn per_fixed(&self) -> Option<u64> {
        (self.fixed > 0).then(|| self.tokens / self.fixed as u64)
    }

    /// The most frequent dismissal reason.
    pub fn reason(&self) -> Option<&str> {
        let most = self.reasons.iter().max_by_key(|(_, n)| **n);
        most.map(|(r, _)| r.as_str())
    }
}

pub type Stats = BTreeMap<(By, String), Row>;

/// The stats of every live run in `records`.
pub fn stats(records: &BTreeMap<(String, String), State>) -> Stats {
    let mut tasks: BTreeMap<&str, Vec<(&str, &State)>> = BTreeMap::new();
    for ((task, run), s) in records {
        tasks.entry(task).or_default().push((run, s));
    }
    let mut rows = Stats::new();
    for runs in tasks.values_mut() {
        runs.sort_by(|a, b| (a.1.at, a.0).cmp(&(b.1.at, b.0)));
        task(runs, &mut rows);
    }
    rows
}

fn task(runs: &[(&str, &State)], rows: &mut Stats) {
    let track = track::track(runs, &BTreeSet::new());
    let n = track.threads.len();
    let (mut keys, mut confirmed) = (vec![BTreeSet::new(); n], vec![false; n]);
    let index = |run: &str, id: &str| {
        let (t, _) = track.of(run, id)?;
        track.threads.iter().position(|u| std::ptr::eq(u, t))
    };
    for (run, s) in runs {
        let mode = s.started.as_ref().map_or("", |x| x.mode.as_str());
        for p in &s.participants {
            let mut add = |by, k: &str| rows.entry((by, k.into())).or_default().tokens += p.tokens;
            add(By::Mode, mode);
            match p.stage {
                Stage::Find => {
                    add(By::Role, &p.role);
                    add(By::Model, &p.model);
                    p.lenses.iter().for_each(|l| add(By::Lens, l));
                }
                Stage::Verify => add(By::Verifier, &p.model),
                _ => {}
            }
        }
        for f in &s.findings {
            let finder = s.participants.iter().find(|p| p.name == f.participant);
            let lenses = match (f.finding.lenses.is_empty(), finder) {
                (true, Some(p)) => p.lenses.clone(),
                _ => f.finding.lenses.clone(),
            };
            let mut mine = vec![(By::Mode, mode.to_string()), (By::Model, f.model.clone())];
            mine.extend(finder.map(|p| (By::Role, p.role.clone())));
            mine.extend(lenses.into_iter().map(|l| (By::Lens, l)));
            match index(run, f.folded.as_deref().unwrap_or(&f.id)) {
                Some(i) => keys[i].extend(mine),
                None => {
                    for k in mine {
                        let r = rows.entry(k).or_default();
                        (r.raised, r.aside) = (r.raised + 1, r.aside + 1);
                    }
                }
            }
        }
        for f in s.heads() {
            let Some((_, model)) = &f.verdict else {
                continue;
            };
            let (i, yes) = (index(run, &f.id), record::verified(f));
            let r = rows.entry((By::Verifier, model.clone())).or_default();
            (r.raised, r.confirmed) = (r.raised + 1, r.confirmed + usize::from(yes));
            match i {
                Some(i) => {
                    confirmed[i] |= yes;
                    count(r, &track.threads[i].fate, yes);
                }
                None => r.aside += 1,
            }
        }
    }
    for (i, t) in track.threads.iter().enumerate() {
        for k in &keys[i] {
            let r = rows.entry(k.clone()).or_default();
            (r.raised, r.confirmed) = (r.raised + 1, r.confirmed + usize::from(confirmed[i]));
            count(r, &t.fate, confirmed[i]);
        }
    }
}

fn count(r: &mut Row, fate: &Fate, confirmed: bool) {
    match fate {
        Fate::Fixed => r.fixed += 1,
        Fate::Dismissed(why) => {
            r.dismissed += 1;
            r.false_confirms += usize::from(confirmed);
            *r.reasons.entry(why.clone()).or_default() += 1;
        }
        Fate::Deferred => r.deferred += 1,
        _ => {}
    }
}

fn pct(a: usize, n: usize) -> usize {
    a * 100 / n.max(1)
}

fn millions(t: u64) -> String {
    format!("{:.1} M", t as f64 / 1e6)
}

/// The one change the numbers suggest, with the number behind it.
pub fn suggest(stats: &Stats) -> Option<String> {
    let rows = |by: By| {
        let mut all: Vec<(&String, &Row)> = (stats.iter())
            .filter(move |((b, _), _)| *b == by)
            .map(|((_, k), r)| (k, r))
            .collect();
        all.retain(|(_, r)| r.raised >= MIN);
        all
    };
    let worst = |by: By, rate: fn(&Row) -> usize| {
        let all = rows(by);
        all.into_iter().max_by_key(|(_, r)| rate(r))
    };
    let false_confirms = |r: &Row| match r.confirmed >= MIN {
        true => pct(r.false_confirms, r.confirmed),
        false => 0,
    };
    if let Some((m, r)) = worst(By::Verifier, false_confirms)
        && false_confirms(r) >= 30
    {
        return Some(format!(
            "Verify on another model than {m}: you dismissed {} of the {} findings it confirmed.",
            r.false_confirms, r.confirmed
        ));
    }
    let dismissed = |r: &Row| pct(r.dismissed, r.raised);
    if let Some((lens, r)) = worst(By::Lens, dismissed)
        && dismissed(r) >= 50
    {
        let why = r.reason().unwrap_or_default();
        return Some(format!(
            "Rewrite or drop the {lens} lens: you dismissed {} of its {} findings, most often as \
             \"{why}\".",
            r.dismissed, r.raised
        ));
    }
    let mut cost: Vec<(&String, u64)> = (rows(By::Model).into_iter())
        .filter_map(|(m, r)| Some((m, r.per_fixed()?)))
        .collect();
    cost.sort_by_key(|c| c.1);
    if let (Some((cheap, c)), Some((dear, d))) = (cost.first(), cost.last())
        && cheap != dear
        && *d >= 2 * c
    {
        return Some(format!(
            "Seat more roles on {cheap}: {dear} spent {} per fixed finding, {cheap} {}.",
            millions(*d),
            millions(*c)
        ));
    }
    let idle = rows(By::Role)
        .into_iter()
        .find(|(_, r)| r.fixed == 0 && r.open() == 0);
    idle.map(|(role, r)| {
        format!(
            "Rewrite or drop the {role} role: none of its {} findings was fixed.",
            r.raised
        )
    })
}

#[cfg(test)]
mod tests;
