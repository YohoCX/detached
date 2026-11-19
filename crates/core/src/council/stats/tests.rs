use super::*;
use crate::council::corpus::Verifier;

/// The records under `tests/fixtures/council`: a review and the gate that re-checked it, with a
/// fix, a dismissal, a deferral and a set-aside finding, and a corpus run.
pub(crate) fn fixture() -> BTreeMap<(String, String), State> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/council");
    let mut all = BTreeMap::new();
    for task in std::fs::read_dir(&root).unwrap() {
        for run in std::fs::read_dir(task.unwrap().path()).unwrap() {
            let dir = run.unwrap().path();
            let events = record::read(&dir).unwrap();
            let lines = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
            assert_eq!(events.len(), lines.lines().count(), "{}", dir.display());
            let name = |p: &std::path::Path| p.file_name().unwrap().to_string_lossy().into_owned();
            let key = (name(dir.parent().unwrap()), name(&dir));
            all.insert(key, State::replay(&events));
        }
    }
    all
}

fn row(stats: &Stats, by: By, key: &str) -> Row {
    stats.get(&(by, key.into())).cloned().unwrap_or_default()
}

#[test]
fn stats_count_each_issue_once_by_role_lens_model_mode_and_verifier() {
    let s = stats(&fixture());
    let bugs = row(&s, By::Role, "bugs");
    assert_eq!(
        (bugs.raised, bugs.fixed, bugs.dismissed, bugs.open()),
        (2, 1, 1, 0)
    );
    assert_eq!((bugs.confirmed, bugs.false_confirms), (2, 1));
    assert_eq!(
        (bugs.tokens, bugs.per_fixed()),
        (1_500_000, Some(1_500_000))
    );
    assert_eq!(bugs.reason(), Some("the name is the API's"));
    let docs = row(&s, By::Role, "docs");
    assert_eq!(
        (
            docs.raised,
            docs.aside,
            docs.deferred,
            docs.fixed,
            docs.confirmed
        ),
        (2, 1, 1, 0, 1)
    );
    assert_eq!(docs.per_fixed(), None);
    // A finding counts under the lenses it reported; tokens under its finder's.
    let (b, i) = (row(&s, By::Lens, "bugs"), row(&s, By::Lens, "impl"));
    assert_eq!((b.raised, b.fixed, i.raised, i.dismissed), (1, 1, 1, 1));
    assert_eq!((b.tokens, i.tokens), (1_500_000, 1_500_000));
    assert_eq!(
        row(&s, By::Model, "opus").tokens,
        1_500_000,
        "no corpus run"
    );
    let review = row(&s, By::Mode, "review");
    assert_eq!(
        (
            review.raised,
            review.fixed,
            review.dismissed,
            review.deferred,
            review.aside
        ),
        (4, 1, 1, 1, 1)
    );
    assert_eq!(
        (review.tokens, row(&s, By::Mode, "gate").tokens),
        (1_600_000, 500_000)
    );
    let (sonnet, opus) = (
        row(&s, By::Verifier, "sonnet"),
        row(&s, By::Verifier, "opus"),
    );
    assert_eq!(
        (
            sonnet.raised,
            sonnet.confirmed,
            sonnet.false_confirms,
            sonnet.fixed
        ),
        (2, 2, 1, 1)
    );
    assert_eq!(
        (opus.raised, opus.confirmed, opus.aside, opus.deferred),
        (2, 1, 1, 1)
    );
}

fn many(by: By, key: &str, r: Row) -> Stats {
    [((by, key.to_string()), r)].into()
}

#[test]
fn one_suggestion_with_the_number_behind_it_and_none_from_few_findings() {
    let s = stats(&fixture());
    assert_eq!(suggest(&s, &[]), None, "too few findings");
    let lens = Row {
        raised: 6,
        dismissed: 4,
        reasons: [("style".to_string(), 3), ("taste".into(), 1)].into(),
        ..Row::default()
    };
    assert_eq!(
        suggest(&many(By::Lens, "docs", lens), &[]).unwrap(),
        "Rewrite or drop the docs lens: you dismissed 4 of its 6 findings, most often as \"style\"."
    );
    let verifier = Row {
        raised: 8,
        confirmed: 6,
        false_confirms: 2,
        ..Row::default()
    };
    assert_eq!(
        suggest(&many(By::Verifier, "haiku", verifier), &[]).unwrap(),
        "Verify on another model than haiku: you dismissed 2 of the 6 findings it confirmed."
    );
    let model = |fixed, tokens| Row {
        raised: 5,
        fixed,
        tokens,
        ..Row::default()
    };
    let mut cost = many(By::Model, "opus", model(1, 4_000_000));
    cost.insert((By::Model, "sonnet".into()), model(2, 2_000_000));
    assert_eq!(
        suggest(&cost, &[]).unwrap(),
        "Seat more roles on sonnet: opus spent 4.0 M per fixed finding, sonnet 1.0 M."
    );
    let role = Row {
        raised: 5,
        dismissed: 5,
        reasons: [("noise".to_string(), 5)].into(),
        ..Row::default()
    };
    assert_eq!(
        suggest(&many(By::Role, "adversarial", role), &[]).unwrap(),
        "Rewrite or drop the adversarial role: none of its 5 findings was fixed."
    );
    let pass = Pass {
        verifiers: [
            (
                "opus".to_string(),
                Verifier {
                    clean: 10,
                    false_confirms: 1,
                    ..Verifier::default()
                },
            ),
            (
                "sonnet".into(),
                Verifier {
                    clean: 8,
                    false_confirms: 4,
                    ..Verifier::default()
                },
            ),
        ]
        .into(),
        ..Pass::default()
    };
    assert_eq!(
        suggest(&many(By::Lens, "docs", Row::default()), &[pass]).unwrap(),
        "Put opus before sonnet in the verifiers: on the corpus's clean subjects sonnet confirmed 4 \
         of 8 majors, opus 1 of 10."
    );
}
