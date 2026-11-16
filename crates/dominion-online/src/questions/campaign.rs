use super::*;
use crate::wire::Reader;

#[derive(Deserialize)]
struct Selection {
    path: Vec<usize>,
    kind: String,
    values: Vec<i32>,
    #[serde(default)]
    top_first: bool,
}
#[derive(Deserialize)]
struct Case {
    label: String,
    hex: String,
    selections: Vec<Selection>,
    expected: Option<Vec<i32>>,
}

#[test]
fn upstream_adversarial_question_campaign() {
    let source = std::env::var("PROTOCOL_QUESTION_FIXTURES")
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|_| include_str!("../../tests/fixtures/questions.json").into());
    let cases: Vec<Case> = serde_json::from_str(&source).unwrap();
    let mut schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../dominion-agent/protocol/agent-view.schema.json"
    ))
    .unwrap();
    schema["$ref"] = "#/$defs/decision".into();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for case in cases {
        let bytes: Vec<_> = (0..case.hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&case.hex[i..i + 2], 16).unwrap())
            .collect();
        let mut reader = Reader::new(&bytes);
        let question = Question::parse(&mut reader).unwrap();
        reader.finish().unwrap();
        let indexed = question
            .indexed(&tests::Context, "campaign", Some(0))
            .unwrap();
        assert_eq!(
            serde_json::to_value(&indexed.decision).unwrap(),
            serde_json::to_value(
                question
                    .project(&tests::Context, "campaign", Some(0))
                    .unwrap()
            )
            .unwrap(),
            "{} projection must be deterministic",
            case.label
        );
        let errors: Vec<_> = validator
            .iter_errors(&serde_json::to_value(&indexed.decision).unwrap())
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", case.label);
        let mut indices = vec![];
        for selection in &case.selections {
            let leaf = indexed
                .leaves
                .iter()
                .position(|(_, path)| *path == selection.path)
                .unwrap();
            let mut values = selection.values.clone();
            if selection.top_first {
                values.reverse();
            }
            let find = |value| {
                indexed
                    .targets
                    .iter()
                    .position(|(group, target)| {
                        *group == leaf
                            && match (selection.kind.as_str(), target) {
                                ("select", Target::Select(v))
                                | ("number", Target::Number(v))
                                | ("name", Target::Name(v)) => *v == value,
                                ("decline", Target::Decline) | ("accept", Target::Accept) => true,
                                _ => false,
                            }
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "{} missing {} target at {:?}",
                            case.label, selection.kind, selection.path
                        )
                    }) as u32
            };
            if selection.kind == "empty" {
                if selection.path.is_empty() {
                    continue;
                }
                indices.push(
                    indexed
                        .targets
                        .iter()
                        .position(|(group, target)| {
                            *group == leaf && matches!(target, Target::Empty)
                        })
                        .unwrap_or_else(|| panic!("{}: no empty branch option", case.label))
                        as u32,
                );
            } else if selection.kind == "decline" || selection.kind == "accept" {
                indices.push(find(0));
            } else {
                indices.extend(values.into_iter().map(find));
            }
        }
        let result = question.encode_answer(
            &tests::Context,
            "campaign",
            &Answer::indices("campaign", indices),
            None,
        );
        match case.expected {
            Some(expected) => assert_eq!(
                result.unwrap_or_else(|e| panic!("{}: {e}", case.label)),
                expected,
                "{}",
                case.label
            ),
            None => assert!(
                result.is_err(),
                "{}: accepted an upstream-invalid selection",
                case.label
            ),
        }
    }
}
