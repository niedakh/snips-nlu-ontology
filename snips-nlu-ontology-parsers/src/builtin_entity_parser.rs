use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::ops::Range;

use itertools::Itertools;
use regex::Regex;

use rustling_converters::{FromRustling, IntoBuiltin};
use nlu_ontology::*;
use nlu_utils::string::{convert_to_byte_range, convert_to_char_index};
use rustling_ontology::{build_parser, OutputKind, Parser, ResolverContext};
use rustling_ontology_moment::Moment;
use chrono::{Local, DateTime, TimeZone};

pub struct BuiltinEntityParser {
    parser: Parser,
    lang: Language,
    reference_datetime: DateTime<Local>,
    supported_entity_kinds: Vec<BuiltinEntityKind>,
}

lazy_static! {
    static ref NON_SPACE_REGEX: Regex = Regex::new(r"[^\s]+").unwrap();
}

lazy_static! {
    static ref NON_SPACE_SEPARATED_LANGUAGES: HashSet<Language> = hashset!(Language::JA);
}

impl BuiltinEntityParser {
    pub fn new(lang: Language, reference_timestamp: i64) -> Self {
        let supported_entity_kinds = BuiltinEntityKind::supported_entity_kinds(lang);
        let ordered_entity_kinds = OutputKind::all()
            .iter()
            .map(|output_kind| output_kind.into_builtin())
            .filter(|builtin_entity_kind| supported_entity_kinds.contains(&builtin_entity_kind))
            .collect();

        let reference_datetime: DateTime<Local> = Local.timestamp(reference_timestamp, 0);

        BuiltinEntityParser {
            parser: build_parser(lang.into_builtin()).unwrap(),
            lang: lang,
            reference_datetime: reference_datetime,
            supported_entity_kinds: ordered_entity_kinds,
        }
    }

    pub fn extract_entities(
        &self,
        sentence: &str,
        filter_entity_kinds: Option<&[BuiltinEntityKind]>,
    ) -> Vec<BuiltinEntity> {
        if NON_SPACE_SEPARATED_LANGUAGES.contains(&self.lang) {
            let original_tokens_bytes_ranges: Vec<Range<usize>> = NON_SPACE_REGEX
                .find_iter(sentence)
                .map(|m| m.start()..m.end())
                .collect();

            let joined_sentence = original_tokens_bytes_ranges
                .iter()
                .map(|r| &sentence[r.clone()])
                .join("");

            if original_tokens_bytes_ranges.is_empty() {
                return vec![];
            }

            let ranges_mapping = get_ranges_mapping(&original_tokens_bytes_ranges);

            let entities = self._extract_entities(&*joined_sentence, filter_entity_kinds);
            entities
                .into_iter()
                .filter_map(|ent| {
                    let byte_range = convert_to_byte_range(&*joined_sentence, &ent.range);
                    let start = byte_range.start;
                    let end = byte_range.end;
                    // Check if match range correspond to original tokens otherwise skip the entity
                    if (start == 0 as usize || ranges_mapping.contains_key(&start))
                        && (ranges_mapping.contains_key(&end))
                    {
                        let start_token_index = if start == 0 as usize {
                            0 as usize
                        } else {
                            ranges_mapping[&start] + 1
                        };
                        let end_token_index = ranges_mapping[&end];

                        let original_start = original_tokens_bytes_ranges[start_token_index].start;
                        let original_end = original_tokens_bytes_ranges[end_token_index].end;
                        let value = sentence[original_start..original_end].to_string();

                        let original_ent = BuiltinEntity {
                            value,
                            range: convert_to_char_index(&sentence, original_start)
                                ..convert_to_char_index(&sentence, original_end),
                            entity: ent.entity,
                            entity_kind: ent.entity_kind,
                        };
                        Some(original_ent)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            self._extract_entities(sentence, filter_entity_kinds)
        }
    }

    fn _extract_entities(
        &self,
        sentence: &str,
        filter_entity_kinds: Option<&[BuiltinEntityKind]>,
    ) -> Vec<BuiltinEntity> {
        // don't cover global nlu_ontology::Grain
        use rustling_ontology::{Grain, Interval};
        let context = ResolverContext::new(
            Interval::starting_at(Moment(self.reference_datetime), Grain::Second)
        );
        let kind_order = self.supported_entity_kinds
            .iter()
            .filter(|entity_kind|
                filter_entity_kinds
                    .map(|kinds| kinds.contains(&entity_kind))
                    .unwrap_or(true))
            .map(|kind| kind.into_builtin())
            .collect::<Vec<OutputKind>>();

        self.parser
            .parse_with_kind_order(&sentence.to_lowercase(), &context, &kind_order)
            .unwrap_or_else(|_| vec![])
            .iter()
            .map(|m|
                BuiltinEntity {
                    value: sentence[m.byte_range.0..m.byte_range.1].into(),
                    range: m.char_range.0..m.char_range.1,
                    entity: m.value.clone().into_builtin(),
                    entity_kind: BuiltinEntityKind::from_rustling(&m.value),
                })
            .sorted_by(|a, b| Ord::cmp(&a.range.start, &b.range.start))
    }
}

fn get_ranges_mapping(tokens_ranges: &Vec<Range<usize>>) -> HashMap<usize, usize> {
    /* Given tokens ranges returns a mapping of byte index to a token index
    The byte indexes corresponds to indexes of the end of tokens in string given by joining all
    the tokens. The tokens index gives the index of the tokens preceding the byte index.

    For instance, if range_mapping[65] -> 5, this means that the token of index 6 starts at the
    65th byte in the joined string
    */
    let ranges_mapping =
        HashMap::<usize, usize>::from_iter(tokens_ranges.iter().enumerate().fold(
            vec![],
            |mut acc: Vec<(usize, usize)>, (token_index, ref original_range)| {
                let previous_end = if token_index == 0 {
                    0 as usize
                } else {
                    acc[acc.len() - 1].0
                };
                acc.push((
                    previous_end + original_range.end - original_range.start,
                    token_index,
                ));
                acc
            },
        ));
    ranges_mapping
}

#[cfg(test)]
mod test {
    use super::*;
    use itertools::Itertools;

    use nlu_ontology::SlotValue::InstantTime;
    use nlu_ontology::language::Language;

    #[test]
    fn test_entities_extraction() {
        // "2013-02-12T04:30:00+00:00"
        let base_timestamp = 1360639800;
        let parser = BuiltinEntityParser::new(Language::EN, base_timestamp);
        assert_eq!(
            vec![BuiltinEntityKind::Number, BuiltinEntityKind::Time],
            parser
                .extract_entities("Book me restaurant for two people tomorrow", None)
                .iter()
                .map(|e| e.entity_kind)
                .collect_vec()
        );

        assert_eq!(
            vec![BuiltinEntityKind::Duration],
            parser
                .extract_entities("The weather during two weeks", None)
                .iter()
                .map(|e| e.entity_kind)
                .collect_vec()
        );

        assert_eq!(
            vec![BuiltinEntityKind::Percentage],
            parser
                .extract_entities("Set light to ten percents", None)
                .iter()
                .map(|e| e.entity_kind)
                .collect_vec()
        );

        assert_eq!(
            vec![BuiltinEntityKind::AmountOfMoney],
            parser
                .extract_entities(
                    "I would like to do a bank transfer of ten euros for my friends",
                    None,
                )
                .iter()
                .map(|e| e.entity_kind)
                .collect_vec()
        );
    }

    #[test]
    fn test_entities_extraction_for_non_space_separated_languages() {
        // "2013-02-12T04:30:00+00:00"
        let base_timestamp = 1360639800;
        let parser = BuiltinEntityParser::new(Language::JA, base_timestamp);
        let expected_time_value = InstantTimeValue {
            value: "2013-02-10 00:00:00 +01:00".to_string(),
            grain: Grain::Day,
            precision: Precision::Exact,
        };

        let expected_entity = BuiltinEntity {
            value: "二 千 十三 年二 月十 日".to_string(),
            range: 10..24,
            entity_kind: BuiltinEntityKind::Time,
            entity: InstantTime(expected_time_value.clone()),
        };

        let parsed_entities = parser.extract_entities(
            " の カリフォル  二 千 十三 年二 月十 日  ニア州の天気予報は？",
            None,
        );
        assert_eq!(1, parsed_entities.len());
        let parsed_entity = &parsed_entities[0];
        assert_eq!(expected_entity.value, parsed_entity.value);
        assert_eq!(expected_entity.range, parsed_entity.range);
        assert_eq!(expected_entity.entity_kind, parsed_entity.entity_kind);

        if let SlotValue::InstantTime(ref parsed_time) = parsed_entity.entity {
            assert_eq!(expected_time_value.grain, parsed_time.grain);
            assert_eq!(expected_time_value.precision, parsed_time.precision);
        } else {
            panic!("")
        }

        assert_eq!(
            Vec::<BuiltinEntity>::new(),
            parser.extract_entities(
                "二 千 十三 年二 月十 日の カリフォルニア州の天気予報は？",
                None,
            )
        );
    }

    #[test]
    fn test_entity_examples_should_be_parsed() {
        // "2013-02-12T04:30:00+00:00"
        let base_timestamp = 1360639800;
        for language in Language::all() {
            let parser = BuiltinEntityParser::new(*language, base_timestamp);
            for entity_kind in BuiltinEntityKind::all() {
                for example in entity_kind.examples(*language) {
                    let results = parser.extract_entities(example, Some(&[*entity_kind]));
                    assert_eq!(
                        1, results.len(),
                        "Expected 1 result for entity kind '{:?}' in language '{:?}' for example \
                        {:?}, but found: {:?}", entity_kind, language, example, results);
                    assert_eq!(example.to_string(), results[0].value);
                }
            }
        }
    }
}
