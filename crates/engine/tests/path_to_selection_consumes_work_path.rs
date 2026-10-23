use photocraft_engine::Session;
use serde_json::json;

/// Converting the Work Path to a selection spends it, as in Photoshop (#306): the Work Path
/// row is gone with its data, the marching ants remain, and one undo step brings both back.
#[test]
fn making_a_selection_consumes_the_work_path_and_undo_restores_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [40, 10], [40, 40], [10, 40]]}]}})).unwrap();
    let r = s.execute("path.toSelection", json!({"name": "work"})).unwrap();
    assert_eq!(r["selected"], json!(true));
    let st = s.active().unwrap();
    assert!(st.doc.work_path.is_none(), "the Work Path is spent by its conversion, as in Photoshop");
    assert!(st.doc.selection.is_some(), "the selection is made");
    s.execute("edit.undo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert!(st.doc.work_path.is_some(), "one undo step restores the Work Path");
    assert!(st.doc.selection.is_none(), "and takes the selection back with it");
    s.execute("edit.redo", json!({})).unwrap();
    let st = s.active().unwrap();
    assert!(st.doc.work_path.is_none(), "the conversion consumed it");
    assert!(st.doc.selection.is_some(), "the ants stay");
}

/// A selection made from a saved path (or a layer-owned path) leaves those paths in place, as
/// in Photoshop: only the Work Path is temporary.
#[test]
fn making_a_selection_from_a_saved_path_keeps_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    s.execute("path.set", json!({"name": "Plate", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [40, 10], [40, 40], [10, 40]]}]}})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[20, 20], [50, 20], [50, 50], [20, 50]]}]}})).unwrap();
    let r = s.execute("path.toSelection", json!({"name": "Plate"})).unwrap();
    assert_eq!(r["selected"], json!(true));
    let st = s.active().unwrap();
    assert_eq!(st.doc.paths.len(), 1, "the saved path survives its conversion");
    assert_eq!(st.doc.paths[0].name, "Plate");
    assert!(st.doc.work_path.is_some(), "and the Work Path stays untouched");
    assert!(st.doc.selection.is_some());
}

/// A conversion from a path with no area (a straight two-point open path rasterizes to nothing)
/// is refused like Photoshop's "empty" alert: the selection is untouched and the path survives
/// so the user can fix it instead of losing it for nothing.
#[test]
fn making_a_selection_from_a_zero_area_path_is_refused_and_keeps_the_path() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"knots": [[12, 52], [52, 20]]}]}})).unwrap();
    assert!(s.execute("path.toSelection", json!({})).is_err(), "a two-point open path has no area");
    let st = s.active().unwrap();
    assert!(st.doc.work_path.is_some(), "the path survives the refused conversion");
    assert!(st.doc.selection.is_none());
    // A closed two-knot subpath is the same nothing; a genuine area makes the selection and
    // spends the Work Path.
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[12, 52], [52, 20]]}]}})).unwrap();
    assert!(s.execute("path.toSelection", json!({})).is_err());
    assert!(s.active().unwrap().doc.work_path.is_some());
    s.execute("path.set", json!({"path": {"subpaths": [{"closed": true, "knots": [[10, 10], [40, 10], [40, 40], [10, 40]]}]}})).unwrap();
    let r = s.execute("path.toSelection", json!({})).unwrap();
    assert_eq!(r["selected"], json!(true));
    let st = s.active().unwrap();
    assert!(st.doc.selection.is_some() && st.doc.work_path.is_none());
}
