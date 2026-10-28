use super::*;

const CLAIMS: &str = include_str!("../../../../tests/fixtures/done/claims.txt");
const NON_CLAIMS: &str = include_str!("../../../../tests/fixtures/done/non-claims.txt");

/// (tag, message) of each block of a list file: blocks end with a `---` line, the first line of a block is its tag.
fn blocks(list: &str) -> Vec<(&str, &str)> {
    list.split("\n---\n")
        .filter_map(|b| b.trim_end().split_once('\n'))
        .map(|(tag, text)| (tag.trim(), text))
        .collect()
}

fn topics(said: &str) -> Vec<Topic> {
    let mut t: Vec<Topic> = read(said).iter().map(|c| c.topic).collect();
    t.sort_by_key(|t| *t == Topic::Build);
    t
}

#[test]
fn every_claim_of_the_list_is_read_with_its_kind() {
    let all = blocks(CLAIMS);
    assert!(all.len() >= 40, "{} claims", all.len());
    let wrong: Vec<String> = all
        .iter()
        .filter(|(tag, text)| {
            let want = match *tag {
                "@test" => vec![Topic::Test],
                "@build" => vec![Topic::Build],
                _ => vec![Topic::Test, Topic::Build],
            };
            topics(text) != want
        })
        .map(|(_, text)| text.chars().take(90).collect())
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} not read as written: {wrong:#?}",
        wrong.len(),
        all.len()
    );
}

#[test]
fn no_message_of_the_non_claim_list_is_read_as_a_claim() {
    let all = blocks(NON_CLAIMS);
    assert!(all.len() >= 60, "{} non-claims", all.len());
    let fired: Vec<String> = all
        .iter()
        .filter_map(|(tag, text)| {
            let got = read(text);
            (!got.is_empty()).then(|| {
                format!(
                    "{tag}: {:?}",
                    got.iter().map(|c| c.shown.clone()).collect::<Vec<_>>()
                )
            })
        })
        .collect();
    assert!(
        fired.is_empty(),
        "{} of {} read as claims: {fired:#?}",
        fired.len(),
        all.len()
    );
}

#[test]
fn a_claim_is_found_in_the_middle_of_a_long_message() {
    let said = "I read the parser.\n\n```\nall tests pass\n```\n\nThen I changed `src/lib.rs`. The build succeeds. Next steps are below.";
    assert_eq!(topics(said), vec![Topic::Build]);
}

#[test]
fn only_the_first_claim_of_a_kind_is_kept() {
    let said = "All tests pass. The tests are green.";
    assert_eq!(read(said).len(), 1);
}
