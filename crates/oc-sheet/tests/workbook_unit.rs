// SPDX-License-Identifier: AGPL-3.0-only
//! Unit tests of the formula workbook (values, cycles, spills, structural rewrites).

use oc_sheet::format::CellValue;
use oc_sheet::formula::workbook::{FixedHost, Workbook};

fn book(cells: &[(&str, &str)]) -> Workbook {
    let mut wb = Workbook::new(Box::new(FixedHost::default()));
    let s = wb.add_sheet("Sheet1");
    for (a, v) in cells {
        let p = oc_sheet::a1::parse_a1(a).unwrap();
        wb.set_contents(s, p.r as u32, p.c as u32, Some(v));
    }
    wb.recompute();
    wb
}

fn val(wb: &Workbook, a: &str) -> CellValue {
    let p = oc_sheet::a1::parse_a1(a).unwrap();
    wb.value(0, p.r as u32, p.c as u32)
}

#[test]
fn basics() {
    let wb = book(&[("A1", "10"), ("A2", "20"), ("A3", "30"), ("B1", "=SUM(A1:A3)"), ("B2", "=A1/0"), ("B3", "=B3"), ("B4", "=NOSUCH(1)")]);
    assert_eq!(val(&wb, "B1"), CellValue::Number(60.0));
    assert_eq!(val(&wb, "B2"), CellValue::Error("#DIV/0!".into()));
    assert_eq!(val(&wb, "B3"), CellValue::Error("#CYCLE!".into()));
    assert_eq!(val(&wb, "B4"), CellValue::Error("#NAME?".into()));
}

#[test]
fn spills() {
    let wb = book(&[("A1", "=SEQUENCE(3)"), ("B1", "=SUM(A1:A3)")]);
    assert_eq!(val(&wb, "A3"), CellValue::Number(3.0));
    assert_eq!(val(&wb, "B1"), CellValue::Number(6.0));
}

/// Apply batches of edits (one `recompute` per batch, like one `hf.batch`) and read cells after the last.
fn batches(steps: &[&[(&str, &str)]]) -> Workbook {
    let mut wb = Workbook::new(Box::new(FixedHost::default()));
    let s = wb.add_sheet("Sheet1");
    for b in steps {
        for (a, v) in *b {
            let p = oc_sheet::a1::parse_a1(a).unwrap();
            wb.set_contents(s, p.r as u32, p.c as u32, if v.is_empty() { None } else { Some(v) });
        }
        wb.recompute();
    }
    wb
}

fn err(e: &str) -> CellValue {
    CellValue::Error(e.into())
}

// The cases below were observed on HyperFormula 3.4.0 through @oc/sheet's engine (one transaction per batch).

#[test]
fn a_blocked_spill_stays_blocked_until_the_formula_is_entered_again() {
    let wb = batches(&[&[("A1", "=SEQUENCE(3)"), ("A2", "x")]]);
    assert_eq!(val(&wb, "A1"), err("#SPILL!"));
    let wb = batches(&[&[("A1", "=SEQUENCE(3)"), ("A2", "x")], &[("A2", "")]]);
    assert_eq!(val(&wb, "A1"), err("#SPILL!"));
    assert_eq!(val(&wb, "A2"), CellValue::Null);
    // Blocker written into a spilled area later: same.
    let wb = batches(&[&[("A1", "=SEQUENCE(3)")], &[("A2", "5")], &[("A2", "")]]);
    assert_eq!(val(&wb, "A1"), err("#SPILL!"));
    // Entering the formula again places it afresh.
    let wb = batches(&[&[("A1", "=SEQUENCE(3)"), ("A2", "x")], &[("A2", "")], &[("A1", "=SEQUENCE(3)")]]);
    assert_eq!((val(&wb, "A1"), val(&wb, "A3")), (CellValue::Number(1.0), CellValue::Number(3.0)));
}

#[test]
fn clearing_a_spilled_cell_is_a_no_op() {
    // What a format change on a spilled cell sends (`setCellContents(addr, null)`).
    let wb = batches(&[&[("A1", "=SEQUENCE(3)")], &[("A2", "")]]);
    assert_eq!((val(&wb, "A1"), val(&wb, "A2")), (CellValue::Number(1.0), CellValue::Number(2.0)));
}

#[test]
fn arrays_on_a_cycle_show_cycle_where_they_claimed_space() {
    let wb = batches(&[&[("A1", "=TRANSPOSE(A1:C1)")]]);
    assert_eq!([val(&wb, "A1"), val(&wb, "A2"), val(&wb, "A3")], [err("#CYCLE!"), err("#CYCLE!"), err("#CYCLE!")]);
    let wb = batches(&[&[("A2", "x"), ("A1", "=TRANSPOSE(A1:C1)")], &[("A2", "")], &[("B1", "5")]]);
    assert_eq!([val(&wb, "A1"), val(&wb, "A2")], [err("#CYCLE!"), CellValue::Null]);
    let wb = batches(&[&[("A1", "=TRANSPOSE(A1:C1)"), ("A2", "x")], &[("A2", "")]]);
    assert_eq!([val(&wb, "A1"), val(&wb, "A2")], [err("#CYCLE!"), CellValue::Null]);
}

#[test]
fn the_older_array_keeps_an_overlapping_area() {
    let wb = batches(&[&[("A2", "=SEQUENCE(1,3)")], &[("B1", "=SEQUENCE(2,1)")]]);
    assert_eq!([val(&wb, "A2"), val(&wb, "B2"), val(&wb, "C2"), val(&wb, "B1")], [
        CellValue::Number(1.0),
        CellValue::Number(2.0),
        CellValue::Number(3.0),
        err("#SPILL!")
    ]);
    let wb = batches(&[&[("B1", "=SEQUENCE(2,1)")], &[("A2", "=SEQUENCE(1,3)")]]);
    assert_eq!([val(&wb, "A2"), val(&wb, "B2"), val(&wb, "C2"), val(&wb, "B1")], [
        err("#SPILL!"),
        CellValue::Number(2.0),
        CellValue::Null,
        CellValue::Number(1.0)
    ]);
}

#[test]
fn structural_edits_keep_claims_and_shrink_on_content() {
    let mut wb = batches(&[&[("A1", "=SEQUENCE(3)"), ("A5", "x")]]);
    wb.add_rows(0, 1, 1).unwrap();
    wb.recompute();
    assert_eq!([val(&wb, "A1"), val(&wb, "A3"), val(&wb, "A6")], [CellValue::Number(1.0), CellValue::Number(3.0), CellValue::String("x".into())]);
    // Deleting rows pulls "x" into the area: #SPILL! for good.
    wb.remove_rows(0, 1, 3).unwrap();
    wb.recompute();
    assert_eq!([val(&wb, "A1"), val(&wb, "A3")], [err("#SPILL!"), CellValue::String("x".into())]);
    wb.set_contents(0, 2, 0, None);
    wb.recompute();
    assert_eq!(val(&wb, "A1"), err("#SPILL!"));
}
