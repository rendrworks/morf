use std::collections::BTreeMap;

use super::*;

fn record(id: &str, label: &str) -> Value {
    Value::Map(BTreeMap::from([
        ("id".to_owned(), Value::String(id.to_owned())),
        ("label".to_owned(), Value::String(label.to_owned())),
    ]))
}

#[test]
fn virtual_list_materializes_only_the_viewport() {
    let model = ListModel::new((0..500).map(|index| Value::Number(index as f64)));
    let mut view = VirtualList::new(40.0, 400.0, 1).unwrap();
    let transitions = view.sync(&model, &[]);

    assert_eq!(view.visible_range(model.len()), 0..12);
    assert_eq!(transitions.len(), 12);
    assert!(
        transitions
            .iter()
            .all(|transition| matches!(transition, ViewTransition::Populate(_)))
    );
}

#[test]
fn virtual_grid_materializes_complete_visible_rows() {
    let model = ListModel::new((0..500).map(|index| Value::Number(index as f64)));
    let mut view = VirtualList::new_grid(50.0, 200.0, 1, 4).unwrap();
    view.set_offset(75.0);
    let transitions = view.sync(&model, &[]);

    assert_eq!(view.visible_range(model.len()), 0..28);
    assert_eq!(transitions.len(), 28);
    assert_eq!(view.columns(), 4);
}

#[test]
fn model_move_marks_target_and_displaced_items() {
    let mut model = ListModel::new((0..20).map(|index| Value::Number(index as f64)));
    let mut view = VirtualList::new(20.0, 200.0, 0).unwrap();
    view.sync(&model, &[]);
    assert!(model.move_item(2, 7));
    let changes = model.take_changes();
    let transitions = view.sync(&model, &changes);

    assert!(transitions.iter().any(|transition| matches!(
        transition,
        ViewTransition::Move { from: 2, item, .. } if item.index == 7
    )));
    assert!(transitions.iter().any(|transition| matches!(
        transition,
        ViewTransition::Displaced { from: 3, item, .. } if item.index == 2
    )));
}

#[test]
fn reconcile_preserves_ids_across_reorder_and_updates() {
    let mut model = ListModel::new([
        record("a", "first"),
        record("b", "second"),
        record("c", "third"),
    ]);
    let a = model.get(0).unwrap().0;
    let b = model.get(1).unwrap().0;
    model.reconcile(
        vec![
            record("b", "changed"),
            record("d", "new"),
            record("a", "first"),
        ],
        Some("id"),
    );

    assert_eq!(model.get(0).unwrap().0, b);
    assert_eq!(model.get(2).unwrap().0, a);
    assert!(matches!(
        model.get(0).unwrap().1,
        Value::Map(value) if value["label"] == Value::String("changed".to_owned())
    ));
    let changes = model.take_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        ListChange::Moved { from: 1, to: 0, id } if *id == b
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        ListChange::Updated { index: 0, id } if *id == b
    )));
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, ListChange::Added { index: 1, .. }))
    );
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, ListChange::Removed { index: 3, .. }))
    );
}
