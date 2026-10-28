use accesskit::{Node, NodeId, Rect, Role, TextDirection, TextPosition, TextSelection};
use std::{
    hash::{Hash, Hasher},
    ops::Range,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub(crate) struct Geometry {
    pub bounds: Rect,
    pub positions: Vec<f32>,
    pub widths: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_selection_keeps_direction_and_real_newline() {
        let document = Document {
            text: "a😀\n雪".into(),
            anchor: 9,
            focus: 1,
            focus_run: None,
            runs: vec![
                Run {
                    display_row: None,
                    source: 0..6,
                    geometry: None,
                },
                Run {
                    display_row: None,
                    source: 6..9,
                    geometry: None,
                },
            ],
        };
        let nodes = document.build_nodes(NodeId(1)).expect("valid document");
        assert_eq!(nodes.children[0].1.value(), Some("a😀\n"));
        assert_eq!(nodes.children[0].1.character_lengths(), &[1, 4, 1]);
        assert_eq!(nodes.selection.anchor.node, nodes.children[1].0);
        assert_eq!(nodes.selection.anchor.character_index, 1);
        assert_eq!(nodes.selection.focus.node, nodes.children[0].0);
        assert_eq!(nodes.selection.focus.character_index, 1);
        assert!(nodes.children[0].1.bounds().is_none());
    }

    #[test]
    fn geometry_changes_do_not_change_identity_or_text() {
        let mut document = Document {
            text: "ab".into(),
            anchor: 1,
            focus: 1,
            focus_run: None,
            runs: vec![Run {
                display_row: None,
                source: 0..2,
                geometry: Some(Geometry {
                    bounds: Rect::new(10., 20., 30., 40.),
                    positions: vec![0., 10.],
                    widths: vec![10., 10.],
                }),
            }],
        };
        let measured = document.build_nodes(NodeId(1)).expect("valid document");
        assert!(measured.children[0].1.bounds().is_some());
        document.runs[0].geometry = None;
        let unmeasured = document.build_nodes(NodeId(1)).expect("valid document");
        assert_eq!(measured.children[0].0, unmeasured.children[0].0);
        assert_eq!(measured.selection, unmeasured.selection);
        assert_eq!(unmeasured.children[0].1.value(), Some("ab"));
        assert!(unmeasured.children[0].1.bounds().is_none());
        document.runs[0].geometry = Some(Geometry {
            bounds: Rect::new(0., 0., 20., 20.),
            positions: vec![10., 0.],
            widths: vec![10., -10.],
        });
        assert!(
            document
                .build_nodes(NodeId(1))
                .expect("text survives bad geometry")
                .children[0]
                .1
                .bounds()
                .is_none()
        );
    }

    #[test]
    fn selectable_clusters_and_crlf_keep_byte_endpoints() {
        let text = "é👩‍💻🇺🇸\r\n";
        let mut document = Document {
            text: text.into(),
            anchor: text.len(),
            focus: 3,
            focus_run: None,
            runs: vec![Run {
                source: 0..text.len(),
                display_row: Some(0),
                geometry: None,
            }],
        };
        let nodes = document
            .build_nodes(NodeId(1))
            .expect("selectable cluster boundaries");
        assert_eq!(nodes.children[0].1.character_lengths(), &[3, 11, 8, 2]);
        assert_eq!(nodes.selection.focus.character_index, 1);
        assert_eq!(nodes.selection.anchor.character_index, 4);
        document.focus = 1;
        assert!(document.build_nodes(NodeId(1)).is_none());
        document.text = format!("a{}", "́".repeat(128));
        document.runs[0].source = 0..document.text.len();
        document.anchor = 0;
        document.focus = 0;
        assert!(
            document.build_nodes(NodeId(1)).is_none(),
            "unrepresentable selectable unit is unavailable"
        );
    }

    #[test]
    fn trailing_empty_run_and_invalid_boundaries() {
        let mut document = Document {
            text: "雪\n".into(),
            anchor: 4,
            focus: 4,
            focus_run: None,
            runs: vec![
                Run {
                    display_row: None,
                    source: 0..4,
                    geometry: None,
                },
                Run {
                    display_row: None,
                    source: 4..4,
                    geometry: None,
                },
            ],
        };
        let nodes = document.build_nodes(NodeId(1)).expect("valid document");
        assert_eq!(nodes.selection.focus.node, nodes.children[1].0);
        assert_eq!(nodes.selection.focus.character_index, 0);
        document.focus = 1;
        assert!(document.build_nodes(NodeId(1)).is_none());
        document.focus = 4;
        document.runs[0].source.end = 3;
        assert!(document.build_nodes(NodeId(1)).is_none());
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub source: Range<usize>,
    pub display_row: Option<u32>,
    pub geometry: Option<Geometry>,
}

#[derive(Clone, Debug)]
pub(crate) struct Document {
    pub text: String,
    pub anchor: usize,
    pub focus: usize,
    pub focus_run: Option<usize>,
    pub runs: Vec<Run>,
}

pub(crate) struct Nodes {
    pub children: Vec<(NodeId, Node)>,
    pub selection: TextSelection,
}

impl Document {
    pub fn build_nodes(&self, parent: NodeId) -> Option<Nodes> {
        if !self.text.is_char_boundary(self.anchor) || !self.text.is_char_boundary(self.focus) {
            return None;
        }
        let mut end = 0;
        let mut children: Vec<(NodeId, Node)> = Vec::new();
        let mut previous_row = None;
        let mut anchor = None;
        let mut focus = None;
        for (index, run) in self.runs.iter().enumerate() {
            if run.source.start != end {
                return None;
            }
            let text = self.text.get(run.source.clone())?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (parent, run.source.start, run.source.end, text).hash(&mut hasher);
            let id = NodeId(hasher.finish());
            let lengths: Vec<u8> = text
                .graphemes(true)
                .map(|character| u8::try_from(character.len()).ok())
                .collect::<Option<_>>()?;
            let mut node = Node::new(Role::TextRun);
            node.set_value(text);
            node.set_character_lengths(lengths.clone());
            if let Some(geometry) = &run.geometry {
                if geometry.positions.len() == lengths.len()
                    && geometry.widths.len() == lengths.len()
                    && geometry.positions.iter().all(|value| value.is_finite())
                    && geometry
                        .widths
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.)
                    && geometry.positions.windows(2).all(|pair| pair[0] <= pair[1])
                    && geometry.bounds.width().is_finite()
                    && geometry.bounds.height().is_finite()
                    && geometry.bounds.width() >= 0.
                    && geometry.bounds.height() > 0.
                {
                    node.set_bounds(geometry.bounds);
                    node.set_text_direction(TextDirection::LeftToRight);
                    node.set_character_positions(geometry.positions.clone());
                    node.set_character_widths(geometry.widths.clone());
                }
            }
            for (offset, position) in [(self.anchor, &mut anchor), (self.focus, &mut focus)] {
                if offset >= run.source.start
                    && offset <= run.source.end
                    && (offset != self.focus
                        || self.focus_run.is_none_or(|focus_run| focus_run == index))
                {
                    let byte_index = offset - run.source.start;
                    let mut boundary = 0;
                    let mut character_index = 0;
                    for length in &lengths {
                        if boundary >= byte_index {
                            break;
                        }
                        boundary += usize::from(*length);
                        character_index += 1;
                    }
                    if boundary != byte_index {
                        return None;
                    }
                    *position = Some(TextPosition {
                        node: id,
                        character_index,
                    });
                }
            }
            if run.display_row.is_some() && run.display_row == previous_row {
                if let Some((previous_id, previous)) = children.last_mut() {
                    previous.set_next_on_line(id);
                    node.set_previous_on_line(*previous_id);
                }
            }
            previous_row = run.display_row;
            end = run.source.end;
            children.push((id, node));
        }
        if end != self.text.len() || children.is_empty() {
            return None;
        }
        Some(Nodes {
            children,
            selection: TextSelection {
                anchor: anchor?,
                focus: focus?,
            },
        })
    }
}
