// SPDX-License-Identifier: Apache-2.0
//! siteReflow.ts parity (`tests/golden/render-svc/reflow.json`) and the port of `siteReflow.test.ts`.

use oc_render_svc::site_mobile::page_items;
use oc_render_svc::site_reflow::{
    Boxed, Item, ReflowItem, ReflowNode, contains, group_rows, mobile_font_size, plan_reflow,
    reading_order, reading_rows, row_reading_order, should_reflow,
};
use oc_schema::primitives::Rect;
use serde_json::{Value, json};

use crate::common::*;

fn rect(v: &Value) -> Rect {
    Rect {
        x: v["x"].as_f64().unwrap(),
        y: v["y"].as_f64().unwrap(),
        w: v["w"].as_f64().unwrap(),
        h: v["h"].as_f64().unwrap(),
    }
}

fn ids<T: ReflowItem>(xs: &[&T]) -> Vec<String> {
    xs.iter().map(|x| x.id().to_string()).collect()
}

fn plan_json<T: ReflowItem>(rows: &[Vec<ReflowNode<'_, T>>]) -> Value {
    Value::Array(rows.iter().map(|r| Value::Array(r.iter().map(|n| json!({ "id": n.item.id(), "box": rect_json(&n.bx), "rows": plan_json(&n.rows) })).collect())).collect())
}

fn rect_json(r: &Rect) -> Value {
    json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h })
}

fn check_layout<T: ReflowItem>(items: &[T], page_w: f64, page_h: f64, want: &Value, label: &str) {
    let refs: Vec<&T> = items.iter().collect();
    let plan = plan_reflow(items, page_w, page_h);
    let got = json!({
        "plan": { "background": ids(&plan.background), "rows": plan_json(&plan.rows) },
        "order": ids(&reading_order(&plan)),
        "readingRows": reading_rows(&refs, 8.0, 0).iter().map(|r| ids(r)).collect::<Vec<_>>(),
        "groupRows": group_rows(&refs).iter().map(|r| ids(r)).collect::<Vec<_>>(),
        "rowOrders": group_rows(&refs).iter().map(|r| ids(&row_reading_order(r))).collect::<Vec<_>>(),
    });
    for k in ["plan", "order", "readingRows", "groupRows", "rowOrders"] {
        if let Some(d) = diff(&got[k], &want[k], &format!("{label}.{k}")) {
            panic!("{d}");
        }
    }
}

fn item_of(v: &Value) -> Item {
    Item {
        id: v["id"].as_str().unwrap().into(),
        bx: rect(&v["box"]),
        z: v["z"].as_f64().unwrap(),
        container: v["container"].as_bool().unwrap_or(false),
        visual: v["visual"].as_bool().unwrap_or(false),
    }
}

#[test]
fn website_pages_match_ts() {
    let g = golden("reflow.json");
    let mut pages = 0;
    for d in g["docs"].as_array().unwrap() {
        let name = d["name"].as_str().unwrap();
        let doc = design(name);
        let (w, h) = (
            d["dims"]["w"].as_f64().unwrap(),
            d["dims"]["h"].as_f64().unwrap(),
        );
        for p in d["pages"].as_array().unwrap() {
            let pid = p["page"].as_str().unwrap();
            let label = format!("{name}/{pid}");
            let items = page_items(&doc, pid);
            let got: Vec<Value> = items
                .iter()
                .map(|it| json!({ "id": it.id, "box": rect_json(&it.bx), "z": it.z, "container": it.container, "visual": it.visual }))
                .collect();
            if let Some(e) = diff(&Value::Array(got), &p["items"], &format!("{label}.items")) {
                panic!("{e}");
            }
            assert_eq!(
                should_reflow(
                    doc.pages[pid].mobile_layout.as_ref().map(|m| m.as_str()),
                    w,
                    h
                ),
                p["shouldReflow"].as_bool().unwrap(),
                "{label}: shouldReflow"
            );
            check_layout(&items, w, h, p, &label);
            pages += 1;
        }
    }
    assert!(pages >= 30, "{pages} pages");
}

#[test]
fn random_layouts_match_ts() {
    let g = golden("reflow.json");
    for (k, c) in g["random"].as_array().unwrap().iter().enumerate() {
        let items: Vec<Item> = c["items"].as_array().unwrap().iter().map(item_of).collect();
        check_layout(
            &items,
            c["page"]["w"].as_f64().unwrap(),
            c["page"]["h"].as_f64().unwrap(),
            c,
            &format!("random[{k}]"),
        );
    }
}

#[test]
fn helper_tables_match_ts() {
    let g = golden("reflow.json");
    for c in g["contains"].as_array().unwrap() {
        assert_eq!(
            contains(&rect(&c["outer"]), &rect(&c["inner"])),
            c["out"].as_bool().unwrap(),
            "contains {c}"
        );
    }
    for c in g["mobileFontSize"].as_array().unwrap() {
        let got = mobile_font_size(c["size"].as_f64().unwrap(), c["pageW"].as_f64().unwrap());
        assert!(
            (got - c["out"].as_f64().unwrap()).abs() < 1e-9,
            "mobileFontSize {c}: {got}"
        );
    }
    for c in g["shouldReflow"].as_array().unwrap() {
        let got = should_reflow(
            c["mode"].as_str(),
            c["dims"]["w"].as_f64().unwrap(),
            c["dims"]["h"].as_f64().unwrap(),
        );
        assert_eq!(got, c["out"].as_bool().unwrap(), "shouldReflow {c}");
    }
}

// ------------------------------------------------------------------ siteReflow.test.ts

fn it_(id: &str, x: f64, y: f64, w: f64, h: f64) -> Item {
    it_z(id, x, y, w, h, 0.0, false)
}

fn it_z(id: &str, x: f64, y: f64, w: f64, h: f64, z: f64, vc: bool) -> Item {
    Item {
        id: id.into(),
        bx: Rect { x, y, w, h },
        z,
        container: vc,
        visual: vc,
    }
}

fn refs<T>(xs: &[T]) -> Vec<&T> {
    xs.iter().collect()
}

fn row_ids<T: ReflowItem + Boxed>(rows: &[Vec<&T>]) -> Vec<Vec<String>> {
    rows.iter().map(|r| ids(r)).collect()
}

#[test]
fn groups_side_by_side_items_into_one_row_and_stacks_rows_top_to_bottom() {
    let items = [
        it_("img", 700.0, 100.0, 600.0, 400.0),
        it_("head", 60.0, 120.0, 560.0, 90.0),
        it_("body", 60.0, 230.0, 520.0, 120.0),
        it_("footer", 60.0, 600.0, 1200.0, 60.0),
    ];
    assert_eq!(
        row_ids(&group_rows(&refs(&items))),
        vec![vec!["img", "head", "body"], vec!["footer"]]
    );
}

#[test]
fn starts_a_new_row_when_vertical_overlap_is_small() {
    let items = [
        it_("a", 0.0, 0.0, 100.0, 100.0),
        it_("b", 200.0, 90.0, 100.0, 100.0),
    ];
    assert_eq!(
        row_ids(&group_rows(&refs(&items))),
        vec![vec!["a"], vec!["b"]]
    );
}

#[test]
fn reads_columns_left_to_right_each_column_top_to_bottom() {
    // Ragged left edges stay in one column.
    let items = [
        it_("img", 700.0, 100.0, 600.0, 400.0),
        it_("button", 58.0, 380.0, 200.0, 60.0),
        it_("head", 60.0, 120.0, 560.0, 90.0),
        it_("body", 62.0, 230.0, 520.0, 120.0),
    ];
    assert_eq!(
        ids(&row_reading_order(&refs(&items))),
        vec!["head", "body", "button", "img"]
    );
}

#[test]
fn keeps_three_cards_in_a_row_in_left_to_right_order() {
    let items = [
        it_("c3", 900.0, 100.0, 300.0, 300.0),
        it_("c1", 60.0, 110.0, 300.0, 300.0),
        it_("c2", 480.0, 95.0, 300.0, 300.0),
    ];
    assert_eq!(
        ids(&row_reading_order(&refs(&items))),
        vec!["c1", "c2", "c3"]
    );
}

#[test]
fn reads_a_three_column_menu_column_by_column_even_when_its_lines_align_across_columns() {
    let mut items = vec![it_("title", 80.0, 60.0, 1206.0, 70.0)];
    for (i, col) in ["a", "b", "c"].iter().enumerate() {
        let x = 80.0 + i as f64 * 413.0;
        items.push(it_(&format!("{col}-head"), x, 220.0, 380.0, 20.0));
        for k in 0..3 {
            let y = 270.0 + k as f64 * 110.0;
            items.push(it_(&format!("{col}{k}-name"), x, y, 300.0, 40.0));
            items.push(it_(
                &format!("{col}{k}-price"),
                x + 300.0,
                y + 2.0,
                80.0,
                36.0,
            ));
            items.push(it_(&format!("{col}{k}-desc"), x, y + 44.0, 300.0, 20.0));
        }
    }
    let order: Vec<String> = reading_rows(&refs(&items), 8.0, 0)
        .iter()
        .flat_map(|r| ids(r))
        .collect();
    assert_eq!(
        &order[..6],
        &[
            "title", "a-head", "a0-name", "a0-price", "a0-desc", "a1-name"
        ]
    );
    let at = |s: &str| order.iter().position(|x| x == s).unwrap();
    assert!(at("a2-desc") < at("b-head"));
    assert!(at("b2-desc") < at("c-head"));
}

#[test]
fn reads_feature_columns_icon_title_text_one_feature_at_a_time() {
    let mut items = vec![it_("heading", 80.0, 120.0, 1206.0, 60.0)];
    for i in 0..3 {
        let x = 80.0 + i as f64 * 420.0;
        items.push(it_(&format!("icon{i}"), x, 300.0, 64.0, 64.0));
        items.push(it_(&format!("title{i}"), x, 390.0, 360.0, 30.0));
        items.push(it_(&format!("text{i}"), x, 430.0, 360.0, 80.0));
    }
    let order: Vec<String> = reading_rows(&refs(&items), 8.0, 0)
        .iter()
        .flat_map(|r| ids(r))
        .collect();
    assert_eq!(
        order,
        vec![
            "heading", "icon0", "title0", "text0", "icon1", "title1", "text1", "icon2", "title2",
            "text2"
        ]
    );
}

#[test]
fn keeps_a_row_of_small_side_by_side_items_together_as_one_row() {
    let items = [
        it_("i1", 400.0, 600.0, 60.0, 60.0),
        it_("i2", 500.0, 600.0, 60.0, 60.0),
        it_("i3", 600.0, 600.0, 60.0, 60.0),
        it_("t", 80.0, 100.0, 600.0, 60.0),
    ];
    assert_eq!(
        row_ids(&reading_rows(&refs(&items), 8.0, 0)),
        vec![vec!["t"], vec!["i1", "i2", "i3"]]
    );
}

const PAGE: (f64, f64) = (1366.0, 768.0);

#[test]
fn turns_full_page_visuals_into_the_background() {
    let items = [
        it_z("bg", 0.0, 0.0, 1366.0, 768.0, 0.0, true),
        it_z("t", 100.0, 100.0, 400.0, 80.0, 1.0, false),
    ];
    let plan = plan_reflow(&items, PAGE.0, PAGE.1);
    assert_eq!(ids(&plan.background), vec!["bg"]);
    assert_eq!(
        plan.rows
            .iter()
            .map(|r| r.iter().map(|n| n.item.id.clone()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec!["t"]]
    );
}

#[test]
fn keeps_button_labels_on_their_button_and_text_on_its_card() {
    let items = [
        it_z("card", 700.0, 100.0, 500.0, 500.0, 0.0, true),
        it_z("cardTitle", 740.0, 140.0, 400.0, 60.0, 1.0, false),
        it_z("btn", 740.0, 500.0, 200.0, 60.0, 2.0, true),
        it_z("btnLabel", 760.0, 512.0, 160.0, 36.0, 3.0, false),
        it_z("head", 60.0, 120.0, 560.0, 90.0, 4.0, false),
    ];
    let plan = plan_reflow(&items, PAGE.0, PAGE.1);
    assert_eq!(plan.rows.len(), 1);
    let (head, card) = (&plan.rows[0][0], &plan.rows[0][1]);
    assert_eq!(head.item.id, "head");
    assert_eq!(card.item.id, "card");
    let flat: Vec<&ReflowNode<'_, Item>> = card.rows.iter().flatten().collect();
    assert_eq!(
        flat.iter().map(|n| n.item.id.as_str()).collect::<Vec<_>>(),
        vec!["cardTitle", "btn"]
    );
    assert_eq!(
        flat[1]
            .rows
            .iter()
            .flatten()
            .map(|n| n.item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["btnLabel"]
    );
    assert_eq!(
        ids(&reading_order(&plan)),
        vec!["head", "card", "cardTitle", "btn", "btnLabel"]
    );
}

#[test]
fn does_not_nest_items_under_containers_drawn_above_them() {
    let items = [
        it_z("text", 100.0, 100.0, 100.0, 40.0, 0.0, false),
        it_z("shape", 50.0, 50.0, 300.0, 200.0, 1.0, true),
    ];
    let plan = plan_reflow(&items, PAGE.0, PAGE.1);
    assert!(plan.rows.iter().flatten().all(|n| n.rows.is_empty()));
}

#[test]
fn containment_needs_most_of_the_inner_box_inside_a_clearly_larger_box() {
    let r = |x, y, w, h| Rect { x, y, w, h };
    assert!(contains(
        &r(0.0, 0.0, 100.0, 100.0),
        &r(10.0, 10.0, 50.0, 50.0)
    ));
    assert!(!contains(
        &r(0.0, 0.0, 100.0, 100.0),
        &r(60.0, 60.0, 80.0, 80.0)
    ));
    assert!(!contains(
        &r(0.0, 0.0, 100.0, 100.0),
        &r(0.0, 0.0, 100.0, 100.0)
    ));
}

#[test]
fn keeps_text_readable_and_headings_in_bounds_on_phones() {
    assert_eq!(mobile_font_size(16.0, 1366.0), 14.0);
    assert_eq!(mobile_font_size(120.0, 1366.0), 44.0);
    let mid = mobile_font_size(64.0, 1366.0);
    assert!(mid > 24.0);
    assert!(mid < 44.0);
}

#[test]
fn reflows_wide_pages_unless_the_page_opts_out_portrait_pages_keep_their_layout() {
    assert!(should_reflow(None, 1366.0, 768.0));
    assert!(should_reflow(Some("auto"), 1366.0, 3000.0));
    assert!(!should_reflow(None, 600.0, 800.0));
    assert!(!should_reflow(Some("desktop"), 1366.0, 768.0));
    assert!(!should_reflow(None, 1080.0, 1920.0));
}
