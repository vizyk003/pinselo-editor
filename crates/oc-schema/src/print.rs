// SPDX-License-Identifier: Apache-2.0
//! Print products (A18): catalogue types and the default catalogue, price calculation, shipping-address
//! validation and print preflight. Every money amount is an integer in the minor unit of the catalogue currency.

use std::collections::HashSet;
use std::sync::LazyLock;

use fancy_regex::Regex;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::color::to_rgba;
use crate::design::DesignDoc;
use crate::elements::{Element, TextEl};
use crate::factory::grid_cell_box;
use crate::js;
use crate::presets::{IN, MM, inch, mm};
use crate::primitives::{ColorValue, Paint, Rect, Unit, Xf};
use crate::svg::re;
use crate::tree::{page_elements, sorted_pages};

// ---------------------------------------------------------------- catalogue types

/// A trim size (or, for apparel/mugs/bags, the print area). px @ 96 dpi like design dims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintSize {
    pub id: String,
    pub label: String,
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintChoice {
    pub id: String,
    pub label: String,
    /// Added per unit (minor units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_unit: Option<f64>,
    /// Multiplies the base price (e.g. 1.25 for double-sided).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
    /// Pages the design needs for this choice (sides).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintOptionGroup {
    pub id: String,
    pub label: String,
    pub choices: Vec<PrintChoice>,
    /// Default choice id (first choice when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Unit price tiers for one size: the tier with the largest `min` ≤ quantity applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceTier {
    pub min: f64,
    pub unit: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GelatoMapping {
    pub product_uid: String,
    pub tokens: IndexMap<String, IndexMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintfulMapping {
    pub variant_key: String,
    pub variants: IndexMap<String, f64>,
    pub placement: String,
    pub technique: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Fulfilment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gelato: Option<GelatoMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printful: Option<PrintfulMapping>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintProduct {
    pub id: String,
    /// 'business-cards' | 'flyers' | 'posters' | 'invitations' | 'postcards' | 'stickers' | 'apparel' | 'drinkware' | 'bags'
    pub category: String,
    pub name: String,
    pub description: String,
    pub sizes: Vec<PrintSize>,
    pub bleed_mm: f64,
    pub safe_mm: f64,
    pub rotatable: bool,
    pub options: Vec<PrintOptionGroup>,
    pub quantities: Vec<f64>,
    pub min_quantity: f64,
    pub max_quantity: f64,
    /// Unit price tiers per size id ('*' = every size).
    pub prices: IndexMap<String, Vec<PriceTier>>,
    pub weight_g: f64,
    pub min_dpi: f64,
    pub good_dpi: f64,
    pub pages: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fulfilment: Option<Fulfilment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneAmounts {
    pub domestic: f64,
    pub international: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShippingMethod {
    pub id: String,
    pub label: String,
    /// Delivery estimate in business days after production.
    pub days: [f64; 2],
    pub base: ZoneAmounts,
    /// Per started kilogram.
    pub per_kg: ZoneAmounts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShippingConfig {
    pub origin: String,
    pub methods: Vec<ShippingMethod>,
    pub countries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxConfig {
    pub rates: IndexMap<String, f64>,
    pub shipping_taxable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintCatalog {
    pub version: f64,
    pub currency: String,
    pub products: Vec<PrintProduct>,
    pub shipping: ShippingConfig,
    pub tax: TaxConfig,
}

// ---------------------------------------------------------------- default catalogue

fn choice(id: &str, label: &str) -> PrintChoice {
    PrintChoice { id: id.into(), label: label.into(), per_unit: None, multiplier: None, pages: None }
}

fn group(id: &str, label: &str, choices: Vec<PrintChoice>, default: Option<&str>) -> PrintOptionGroup {
    PrintOptionGroup { id: id.into(), label: label.into(), choices, default: default.map(String::from) }
}

fn paper(std: &str, premium: &str, premium_multiplier: f64) -> PrintOptionGroup {
    group(
        "paper",
        "Paper",
        vec![choice("standard", std), PrintChoice { multiplier: Some(premium_multiplier), ..choice("premium", premium) }],
        None,
    )
}

fn sides(back_multiplier: f64) -> PrintOptionGroup {
    group(
        "sides",
        "Sides",
        vec![
            PrintChoice { pages: Some(1.0), ..choice("1", "Front only") },
            PrintChoice { pages: Some(2.0), multiplier: Some(back_multiplier), ..choice("2", "Front and back") },
        ],
        None,
    )
}

const COUNTRIES: &[&str] = &[
    "US", "CA", "GB", "IE", "DE", "FR", "NL", "BE", "LU", "AT", "CH", "IT", "ES", "PT", "DK", "SE", "NO", "FI", "PL", "CZ",
    "SK", "HU", "SI", "HR", "RO", "BG", "GR", "EE", "LV", "LT", "AU", "NZ", "JP", "SG", "HK", "KR", "IN", "BR", "MX",
    "AE", "IL", "ZA",
];

fn size(id: &str, label: &str, w: f64, h: f64, unit: Unit) -> PrintSize {
    PrintSize { id: id.into(), label: label.into(), w, h, unit }
}

fn tiers(t: &[(f64, f64)]) -> Vec<PriceTier> {
    t.iter().map(|(min, unit)| PriceTier { min: *min, unit: *unit }).collect()
}

fn prices(p: &[(&str, &[(f64, f64)])]) -> IndexMap<String, Vec<PriceTier>> {
    p.iter().map(|(k, t)| ((*k).to_string(), tiers(t))).collect()
}

fn tokens(t: &[(&str, &[(&str, &str)])]) -> IndexMap<String, IndexMap<String, String>> {
    t.iter()
        .map(|(k, m)| ((*k).to_string(), m.iter().map(|(a, b)| ((*a).to_string(), (*b).to_string())).collect()))
        .collect()
}

fn gelato(uid: &str, t: &[(&str, &[(&str, &str)])]) -> Option<GelatoMapping> {
    Some(GelatoMapping { product_uid: uid.into(), tokens: tokens(t) })
}

fn printful(key: &str, variants: &[(&str, f64)], placement: &str, technique: &str) -> Option<PrintfulMapping> {
    Some(PrintfulMapping {
        variant_key: key.into(),
        variants: variants.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(),
        placement: placement.into(),
        technique: technique.into(),
    })
}

const SIDES_TOKENS: (&str, &[(&str, &str)]) = ("sides", &[("1", "4-0"), ("2", "4-4")]);

#[allow(clippy::too_many_arguments)]
fn product(
    id: &str,
    category: &str,
    name: &str,
    description: &str,
    sizes: Vec<PrintSize>,
    (bleed_mm, safe_mm, rotatable): (f64, f64, bool),
    options: Vec<PrintOptionGroup>,
    (quantities, min_quantity, max_quantity): (&[f64], f64, f64),
    prices: IndexMap<String, Vec<PriceTier>>,
    (weight_g, min_dpi, good_dpi): (f64, f64, f64),
    fulfilment: Fulfilment,
) -> PrintProduct {
    PrintProduct {
        id: id.into(),
        category: category.into(),
        name: name.into(),
        description: description.into(),
        sizes,
        bleed_mm,
        safe_mm,
        rotatable,
        options,
        quantities: quantities.to_vec(),
        min_quantity,
        max_quantity,
        prices,
        weight_g,
        min_dpi,
        good_dpi,
        pages: 1.0,
        note: None,
        fulfilment: Some(fulfilment),
    }
}

pub static DEFAULT_PRINT_CATALOG: LazyLock<PrintCatalog> = LazyLock::new(|| {
    let letter_tiers: &[(f64, f64)] = &[(10.0, 95.0), (50.0, 60.0), (100.0, 42.0), (250.0, 28.0), (500.0, 20.0), (1000.0, 15.0)];
    let products = vec![
        product(
            "business-cards",
            "business-cards",
            "Business cards",
            "Crisp cards on thick stock. Standard 350 gsm silk or premium 450 gsm soft-touch.",
            vec![size("us", "US (3.5 × 2 in)", inch(3.5), inch(2.0), Unit::In), size("eu", "EU (85 × 55 mm)", mm(85.0), mm(55.0), Unit::Mm)],
            (3.0, 3.0, true),
            vec![paper("Standard — 350 gsm silk", "Premium — 450 gsm soft-touch", 1.5), sides(1.25)],
            (&[50.0, 100.0, 250.0, 500.0, 1000.0], 50.0, 5000.0),
            prices(&[("*", &[(50.0, 40.0), (100.0, 25.0), (250.0, 16.0), (500.0, 11.0), (1000.0, 8.0)])]),
            (4.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato(
                    "cards_pf_{size}_pt_{paper}_cl_{sides}_{orientation}",
                    &[
                        ("size", &[("us", "bx"), ("eu", "bc")]),
                        ("paper", &[("standard", "350-gsm-coated-silk"), ("premium", "450-gsm-soft-touch")]),
                        SIDES_TOKENS,
                    ],
                ),
                printful: None,
            },
        ),
        product(
            "flyers",
            "flyers",
            "Flyers",
            "Single sheets for events, menus and promotions.",
            vec![
                size("a5", "A5 (148 × 210 mm)", mm(148.0), mm(210.0), Unit::Mm),
                size("a4", "A4 (210 × 297 mm)", mm(210.0), mm(297.0), Unit::Mm),
                size("letter", "US Letter (8.5 × 11 in)", inch(8.5), inch(11.0), Unit::In),
            ],
            (3.0, 5.0, true),
            vec![paper("Standard — 170 gsm silk", "Premium — 250 gsm matte", 1.35), sides(1.2)],
            (&[25.0, 50.0, 100.0, 250.0, 500.0, 1000.0], 10.0, 10000.0),
            prices(&[
                ("a5", &[(10.0, 60.0), (50.0, 38.0), (100.0, 26.0), (250.0, 17.0), (500.0, 12.0), (1000.0, 9.0)]),
                ("a4", letter_tiers),
                ("letter", letter_tiers),
            ]),
            (10.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato(
                    "flyers_pf_{size}_pt_{paper}_cl_{sides}_{orientation}",
                    &[
                        ("size", &[("a5", "a5"), ("a4", "a4"), ("letter", "us-letter")]),
                        ("paper", &[("standard", "170-gsm-coated-silk"), ("premium", "250-gsm-uncoated")]),
                        SIDES_TOKENS,
                    ],
                ),
                printful: None,
            },
        ),
        product(
            "posters",
            "posters",
            "Posters",
            "Large-format prints on poster paper.",
            vec![
                size("18x24", "18 × 24 in", inch(18.0), inch(24.0), Unit::In),
                size("a2", "A2 (420 × 594 mm)", mm(420.0), mm(594.0), Unit::Mm),
                size("a3", "A3 (297 × 420 mm)", mm(297.0), mm(420.0), Unit::Mm),
            ],
            (3.0, 10.0, true),
            vec![paper("Matte — 200 gsm", "Satin — 250 gsm premium", 1.3)],
            (&[1.0, 2.0, 5.0, 10.0, 25.0], 1.0, 500.0),
            prices(&[
                ("18x24", &[(1.0, 1900.0), (5.0, 1500.0), (10.0, 1200.0), (25.0, 950.0)]),
                ("a2", &[(1.0, 1700.0), (5.0, 1350.0), (10.0, 1100.0), (25.0, 880.0)]),
                ("a3", &[(1.0, 1100.0), (5.0, 850.0), (10.0, 700.0), (25.0, 560.0)]),
            ]),
            (120.0, 100.0, 150.0),
            Fulfilment {
                gelato: gelato(
                    "flat_{size}_pf_{paper}_4-0_{orientation}",
                    &[
                        (
                            "size",
                            &[("18x24", "18x24-inch-457x610-mm"), ("a2", "420x594-mm-16x24-inch"), ("a3", "297x420-mm-12x16-inch")],
                        ),
                        ("paper", &[("standard", "200-gsm-80lb-uncoated"), ("premium", "250-gsm-100lb-uncoated-offwhite-archival")]),
                    ],
                ),
                printful: printful("{size}", &[("18x24", 1.0)], "default", "digital"),
            },
        ),
        product(
            "invitations",
            "invitations",
            "Invitations",
            "5 × 7 in cards for weddings, parties and events, with matching envelopes.",
            vec![size("5x7", "5 × 7 in", inch(5.0), inch(7.0), Unit::In)],
            (3.0, 5.0, true),
            vec![
                paper("Standard — 350 gsm silk", "Premium — 400 gsm textured", 1.35),
                sides(1.2),
                group(
                    "envelopes",
                    "Envelopes",
                    vec![
                        choice("none", "No envelopes"),
                        PrintChoice { per_unit: Some(25.0), ..choice("white", "White envelopes") },
                        PrintChoice { per_unit: Some(35.0), ..choice("kraft", "Kraft envelopes") },
                    ],
                    Some("white"),
                ),
            ],
            (&[10.0, 25.0, 50.0, 100.0, 200.0], 10.0, 2000.0),
            prices(&[("*", &[(10.0, 220.0), (25.0, 170.0), (50.0, 130.0), (100.0, 100.0), (200.0, 80.0)])]),
            (25.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato(
                    "cards_pf_5x7-inch_pt_{paper}_cl_{sides}_{orientation}",
                    &[("paper", &[("standard", "350-gsm-coated-silk"), ("premium", "324-gsm-uncoated-natural")]), SIDES_TOKENS],
                ),
                printful: None,
            },
        ),
        product(
            "postcards",
            "postcards",
            "Postcards",
            "Mail-ready cards with room for an address on the back.",
            vec![size("6x4", "6 × 4 in", inch(6.0), inch(4.0), Unit::In), size("a6", "A6 (148 × 105 mm)", mm(148.0), mm(105.0), Unit::Mm)],
            (3.0, 4.0, true),
            vec![paper("Standard — 350 gsm silk", "Premium — 400 gsm matte", 1.3), sides(1.15)],
            (&[10.0, 25.0, 50.0, 100.0, 250.0, 500.0], 10.0, 5000.0),
            prices(&[("*", &[(10.0, 120.0), (25.0, 85.0), (50.0, 60.0), (100.0, 42.0), (250.0, 30.0), (500.0, 22.0)])]),
            (8.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato(
                    "cards_pf_{size}_pt_{paper}_cl_{sides}_{orientation}",
                    &[
                        ("size", &[("6x4", "4x6-inch"), ("a6", "a6")]),
                        ("paper", &[("standard", "350-gsm-coated-silk"), ("premium", "350-gsm-uncoated")]),
                        SIDES_TOKENS,
                    ],
                ),
                printful: None,
            },
        ),
        PrintProduct {
            note: Some(
                "Contour (die-cut) cutting along your artwork is not available yet; choose the shape closest to your design."
                    .into(),
            ),
            ..product(
                "stickers",
                "stickers",
                "Stickers",
                "Waterproof vinyl stickers, cut square, round or with rounded corners.",
                vec![
                    size("2in", "2 × 2 in", inch(2.0), inch(2.0), Unit::In),
                    size("3in", "3 × 3 in", inch(3.0), inch(3.0), Unit::In),
                    size("4in", "4 × 4 in", inch(4.0), inch(4.0), Unit::In),
                ],
                (2.0, 3.0, false),
                vec![
                    group(
                        "shape",
                        "Cut shape",
                        vec![choice("square", "Square"), choice("rounded", "Rounded corners"), choice("circle", "Circle")],
                        None,
                    ),
                    group(
                        "material",
                        "Material",
                        vec![choice("matte", "Matte vinyl"), PrintChoice { multiplier: Some(1.1), ..choice("glossy", "Glossy vinyl") }],
                        None,
                    ),
                ],
                (&[10.0, 25.0, 50.0, 100.0, 250.0, 500.0], 10.0, 5000.0),
                prices(&[
                    ("2in", &[(10.0, 90.0), (50.0, 55.0), (100.0, 38.0), (250.0, 26.0), (500.0, 19.0)]),
                    ("3in", &[(10.0, 120.0), (50.0, 75.0), (100.0, 52.0), (250.0, 36.0), (500.0, 27.0)]),
                    ("4in", &[(10.0, 160.0), (50.0, 100.0), (100.0, 70.0), (250.0, 50.0), (500.0, 38.0)]),
                ]),
                (3.0, 150.0, 300.0),
                Fulfilment { gelato: None, printful: printful("{size}", &[("3in", 10163.0), ("4in", 10164.0)], "default", "digital") },
            )
        },
        product(
            "t-shirts",
            "apparel",
            "T-shirts",
            "Unisex cotton tees printed on the front (direct-to-garment). Your design is the print area.",
            vec![size("front", "Front print area (12 × 16 in)", inch(12.0), inch(16.0), Unit::In)],
            (0.0, 0.0, false),
            vec![
                group(
                    "garment",
                    "Shirt size",
                    ["S", "M", "L", "XL", "2XL"]
                        .iter()
                        .map(|s| PrintChoice { per_unit: (*s == "2XL").then_some(200.0), ..choice(s, s) })
                        .collect(),
                    Some("M"),
                ),
                group("color", "Shirt colour", vec![choice("white", "White"), PrintChoice { per_unit: Some(100.0), ..choice("black", "Black") }], None),
            ],
            (&[1.0, 2.0, 5.0, 10.0, 25.0], 1.0, 500.0),
            prices(&[("*", &[(1.0, 2200.0), (5.0, 1900.0), (10.0, 1700.0), (25.0, 1500.0)])]),
            (180.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato(
                    "apparel_product_gca_t-shirt_gsc_crewneck_gcu_unisex_gqa_classic_gsi_{garment}_gco_{color}_gpr_4-0",
                    &[
                        ("garment", &[("S", "s"), ("M", "m"), ("L", "l"), ("XL", "xl"), ("2XL", "2xl")]),
                        ("color", &[("white", "white"), ("black", "black")]),
                    ],
                ),
                printful: printful(
                    "{garment}|{color}",
                    &[
                        ("S|white", 4011.0),
                        ("M|white", 4012.0),
                        ("L|white", 4013.0),
                        ("XL|white", 4014.0),
                        ("2XL|white", 4015.0),
                        ("S|black", 4016.0),
                        ("M|black", 4017.0),
                        ("L|black", 4018.0),
                        ("XL|black", 4019.0),
                        ("2XL|black", 4020.0),
                    ],
                    "front",
                    "dtg",
                ),
            },
        ),
        product(
            "mugs",
            "drinkware",
            "Mugs",
            "11 oz white ceramic mugs, printed all the way around. Your design is the wrap area.",
            vec![size("11oz", "11 oz wrap (9 × 3.5 in)", inch(9.0), inch(3.5), Unit::In)],
            (0.0, 6.0, false),
            vec![],
            (&[1.0, 2.0, 4.0, 6.0, 12.0, 24.0], 1.0, 500.0),
            prices(&[("*", &[(1.0, 1500.0), (6.0, 1300.0), (12.0, 1100.0), (24.0, 950.0)])]),
            (400.0, 150.0, 300.0),
            Fulfilment {
                gelato: gelato("mug_product_msz_11-oz_mmat_ceramic-white_cl_4-0", &[]),
                printful: printful("{size}", &[("11oz", 1320.0)], "default", "sublimation"),
            },
        ),
        product(
            "tote-bags",
            "bags",
            "Tote bags",
            "Natural cotton totes printed on one side. Your design is the print area.",
            vec![size("front", "Print area (14 × 14 in)", inch(14.0), inch(14.0), Unit::In)],
            (0.0, 0.0, false),
            vec![group("color", "Bag colour", vec![choice("natural", "Natural"), PrintChoice { per_unit: Some(100.0), ..choice("black", "Black") }], None)],
            (&[1.0, 5.0, 10.0, 25.0, 50.0], 1.0, 500.0),
            prices(&[("*", &[(1.0, 1800.0), (10.0, 1500.0), (25.0, 1250.0), (50.0, 1050.0)])]),
            (150.0, 150.0, 300.0),
            Fulfilment { gelato: None, printful: printful("{color}", &[("natural", 10457.0), ("black", 10458.0)], "front", "dtg") },
        ),
    ];
    let zone = |domestic, international| ZoneAmounts { domestic, international };
    let rates: &[(&str, f64)] = &[
        ("GB", 20.0), ("IE", 23.0), ("DE", 19.0), ("FR", 20.0), ("NL", 21.0), ("BE", 21.0), ("LU", 17.0), ("AT", 20.0),
        ("IT", 22.0), ("ES", 21.0), ("PT", 23.0), ("DK", 25.0), ("SE", 25.0), ("FI", 25.5), ("PL", 23.0), ("CZ", 21.0),
        ("SK", 23.0), ("HU", 27.0), ("SI", 22.0), ("HR", 25.0), ("RO", 19.0), ("BG", 20.0), ("GR", 24.0), ("EE", 22.0),
        ("LV", 21.0), ("LT", 21.0), ("NO", 25.0), ("CH", 8.1), ("AU", 10.0), ("NZ", 15.0), ("JP", 10.0), ("SG", 9.0),
        ("CA", 5.0), ("CA-ON", 13.0), ("CA-QC", 14.975), ("CA-BC", 12.0), ("IN", 18.0), ("ZA", 15.0), ("MX", 16.0),
        ("BR", 0.0), ("AE", 5.0), ("US-CA", 7.25), ("US-NY", 4.0), ("US-TX", 6.25), ("US-FL", 6.0), ("US-WA", 6.5),
        ("US-IL", 6.25), ("US-PA", 6.0),
    ];
    PrintCatalog {
        version: 1.0,
        currency: String::new(),
        products,
        shipping: ShippingConfig {
            origin: "US".into(),
            methods: vec![
                ShippingMethod {
                    id: "standard".into(),
                    label: "Standard".into(),
                    days: [5.0, 8.0],
                    base: zone(499.0, 1299.0),
                    per_kg: zone(150.0, 600.0),
                },
                ShippingMethod {
                    id: "express".into(),
                    label: "Express".into(),
                    days: [2.0, 3.0],
                    base: zone(1499.0, 3499.0),
                    per_kg: zone(300.0, 1200.0),
                },
            ],
            countries: COUNTRIES.iter().map(|s| (*s).to_string()).collect(),
        },
        tax: TaxConfig { rates: rates.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(), shipping_taxable: true },
    }
});

/// Error with a stable `code` and an English message (same as the TS `PrintError`).
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct PrintError {
    pub code: String,
    pub message: String,
}

impl PrintError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        PrintError { code: code.into(), message: message.into() }
    }
}

/// Validate the shape of an operator catalogue (the message names the first problem).
pub fn check_print_catalog(c: &PrintCatalog) -> Result<(), String> {
    if c.products.is_empty() {
        return Err("catalog has no products".into());
    }
    let mut ids: HashSet<&str> = HashSet::new();
    for p in &c.products {
        if p.id.is_empty() || ids.contains(p.id.as_str()) {
            return Err(format!("duplicate or empty product id {}", p.id));
        }
        ids.insert(&p.id);
        if p.sizes.is_empty() {
            return Err(format!("{}: no sizes", p.id));
        }
        for s in &p.sizes {
            if !(s.w > 0.0 && s.h > 0.0) {
                return Err(format!("{}/{}: bad size", p.id, s.id));
            }
        }
        for s in &p.sizes {
            if p.prices.get(&s.id).or_else(|| p.prices.get("*")).is_none_or(Vec::is_empty) {
                return Err(format!("{}/{}: no price tiers", p.id, s.id));
            }
        }
        if !(p.min_quantity >= 1.0 && p.max_quantity >= p.min_quantity) {
            return Err(format!("{}: bad quantity range", p.id));
        }
    }
    if c.shipping.methods.is_empty() {
        return Err("catalog has no shipping methods".into());
    }
    Ok(())
}

// ---------------------------------------------------------------- selection & price

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintSelection {
    pub product_id: String,
    pub size_id: String,
    /// option group id → choice id (defaults fill the gaps).
    pub options: IndexMap<String, String>,
    pub quantity: f64,
}

pub fn find_product<'a>(catalog: &'a PrintCatalog, id: &str) -> Option<&'a PrintProduct> {
    catalog.products.iter().find(|p| p.id == id)
}

/// Selected choice per option group, defaults applied; unknown choices are an error.
pub fn resolve_options<'a>(p: &'a PrintProduct, picked: &IndexMap<String, String>) -> Result<IndexMap<String, &'a PrintChoice>, PrintError> {
    let mut out = IndexMap::new();
    for g in &p.options {
        let id: Option<&str> = picked
            .get(&g.id)
            .map(String::as_str)
            .or(g.default.as_deref())
            .or_else(|| g.choices.first().map(|c| c.id.as_str()));
        let Some(c) = g.choices.iter().find(|x| Some(x.id.as_str()) == id) else {
            return Err(PrintError::new(
                "bad_option",
                format!("Unknown {} “{}”.", g.label.to_lowercase(), id.unwrap_or("undefined")),
            ));
        };
        out.insert(g.id.clone(), c);
    }
    Ok(out)
}

/// Pages the design needs for this selection.
pub fn expected_pages(p: &PrintProduct, picked: &IndexMap<String, String>) -> Result<f64, PrintError> {
    let opts = resolve_options(p, picked)?;
    Ok(opts.values().find_map(|c| c.pages.filter(|n| *n != 0.0 && !n.is_nan())).unwrap_or(p.pages))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceLine {
    /// 'product' | 'option' | 'shipping' | 'tax'
    pub kind: String,
    pub label: String,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceBreakdown {
    pub currency: String,
    pub unit_price: f64,
    pub subtotal: f64,
    pub shipping: f64,
    pub tax: f64,
    pub tax_rate: f64,
    pub total: f64,
    pub weight_g: f64,
    pub lines: Vec<PriceLine>,
    pub shipping_method: String,
    pub delivery_days: Option<[f64; 2]>,
}

pub fn unit_price_for(p: &PrintProduct, size_id: &str, quantity: f64) -> Result<f64, PrintError> {
    let mut t: Vec<&PriceTier> = p.prices.get(size_id).or_else(|| p.prices.get("*")).map(|v| v.iter().collect()).unwrap_or_default();
    t.sort_by(|a, b| a.min.partial_cmp(&b.min).unwrap_or(std::cmp::Ordering::Equal));
    let Some(first) = t.first() else {
        return Err(PrintError::new("no_price", format!("No price for {} ({size_id}).", p.name)));
    };
    let mut unit = first.unit;
    for x in &t {
        if quantity >= x.min {
            unit = x.unit;
        }
    }
    Ok(unit)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Destination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Price a selection (see the TS doc comment for the formula).
pub fn quote_print(
    catalog: &PrintCatalog,
    sel: &PrintSelection,
    dest: &Destination,
    shipping_method: Option<&str>,
    currency: Option<&str>,
) -> Result<PriceBreakdown, PrintError> {
    let currency = currency
        .map(String::from)
        .unwrap_or_else(|| if catalog.currency.is_empty() { "usd".into() } else { catalog.currency.clone() });
    let p = find_product(catalog, &sel.product_id).ok_or_else(|| PrintError::new("unknown_product", "Unknown product."))?;
    let size = p.sizes.iter().find(|s| s.id == sel.size_id).ok_or_else(|| PrintError::new("unknown_size", "Unknown size."))?;
    let q = sel.quantity.floor();
    if !(q >= p.min_quantity && q <= p.max_quantity) {
        return Err(PrintError::new(
            "bad_quantity",
            format!("Order between {} and {}.", js::num_to_string(p.min_quantity), js::num_to_string(p.max_quantity)),
        ));
    }
    let opts = resolve_options(p, &sel.options)?;
    let unit = unit_price_for(p, &size.id, q)?;
    let multiplier = opts.values().fold(1.0, |m, c| m * c.multiplier.unwrap_or(1.0));
    let mut lines = Vec::new();
    let base = js::round(q * unit * multiplier);
    lines.push(PriceLine {
        kind: "product".into(),
        label: format!("{} — {} × {}", p.name, size.label, js::num_to_string(q)),
        amount: base,
    });
    let mut subtotal = base;
    for (gid, c) in &opts {
        let Some(pu) = c.per_unit.filter(|v| *v != 0.0 && !v.is_nan()) else { continue };
        let amt = pu * q;
        subtotal += amt;
        let glabel = p.options.iter().find(|g| &g.id == gid).map_or(gid.as_str(), |g| g.label.as_str());
        lines.push(PriceLine { kind: "option".into(), label: format!("{glabel}: {}", c.label), amount: amt });
    }
    let weight_g = p.weight_g * q;
    let wanted = shipping_method.map(String::from).unwrap_or_else(|| catalog.shipping.methods[0].id.clone());
    let method = catalog
        .shipping
        .methods
        .iter()
        .find(|m| m.id == wanted)
        .ok_or_else(|| PrintError::new("bad_shipping", "Unknown shipping method."))?;
    let country = dest.country.as_deref().filter(|c| !c.is_empty()).map(str::to_uppercase);
    let intl = country.as_deref().is_some_and(|c| c != catalog.shipping.origin.to_uppercase());
    let (b, per) = if intl { (method.base.international, method.per_kg.international) } else { (method.base.domestic, method.per_kg.domestic) };
    let shipping = b + (weight_g / 1000.0).ceil() * per;
    lines.push(PriceLine { kind: "shipping".into(), label: format!("{} shipping", method.label), amount: shipping });
    let tax_rate = match &country {
        Some(c) => {
            let st = format!("{c}-{}", dest.state.as_deref().unwrap_or("").to_uppercase());
            catalog.tax.rates.get(&st).or_else(|| catalog.tax.rates.get(c)).copied().unwrap_or(0.0)
        }
        None => 0.0,
    };
    let tax = js::round(((subtotal + if catalog.tax.shipping_taxable { shipping } else { 0.0 }) * tax_rate) / 100.0);
    if tax != 0.0 && !tax.is_nan() {
        lines.push(PriceLine { kind: "tax".into(), label: format!("Estimated tax ({}%)", js::num_to_string(tax_rate)), amount: tax });
    }
    Ok(PriceBreakdown {
        currency,
        unit_price: unit,
        subtotal,
        shipping,
        tax,
        tax_rate,
        total: subtotal + shipping + tax,
        weight_g,
        lines,
        shipping_method: method.id.clone(),
        delivery_days: Some(method.days),
    })
}

static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| re(r"\{([A-Za-z0-9_]+)\}"));

/// Fill a `{token}` template from the selection (size id, option choice ids, orientation).
pub fn fill_template(
    tpl: &str,
    vars: &IndexMap<String, String>,
    tokens: Option<&IndexMap<String, IndexMap<String, String>>>,
) -> Result<String, PrintError> {
    let mut out = String::new();
    let mut last = 0;
    for caps in TOKEN_RE.captures_iter(tpl) {
        let Ok(caps) = caps else { break };
        let m = caps.get(0).expect("group 0");
        let k = &caps[1];
        let Some(raw) = vars.get(k) else {
            return Err(PrintError::new("mapping", format!("Missing value for {{{k}}}.")));
        };
        out.push_str(&tpl[last..m.start()]);
        out.push_str(tokens.and_then(|t| t.get(k)).and_then(|t| t.get(raw)).unwrap_or(raw));
        last = m.end();
    }
    out.push_str(&tpl[last..]);
    Ok(out)
}

// ---------------------------------------------------------------- shipping addresses

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShippingAddress {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    pub line1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line2: Option<String>,
    pub city: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub postal_code: String,
    /// ISO 3166-1 alpha-2.
    pub country: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddressCheck {
    pub ok: bool,
    pub address: ShippingAddress,
    /// field → 'required' | 'invalid' | 'unsupported' | 'too_long'
    pub errors: IndexMap<String, String>,
}

struct AddressRes {
    ws: Regex,
    country: Regex,
    phone: Regex,
    email: Regex,
    postcode: Vec<(&'static str, Regex)>,
}

static ADDRESS: LazyLock<AddressRes> = LazyLock::new(|| AddressRes {
    ws: re(r"\s+"),
    country: re(r"^[A-Z]{2}$"),
    phone: re(r"^\+?[0-9\s().-]{6,20}$"),
    email: re(r"^[^\s@]+@[^\s@]+\.[^\s@]+$"),
    postcode: [
        ("US", r"^[0-9]{5}(-[0-9]{4})?$"),
        ("CA", r"^[A-Z][0-9][A-Z] ?[0-9][A-Z][0-9]$"),
        ("GB", r"^[A-Z]{1,2}[0-9][A-Z0-9]? ?[0-9][A-Z]{2}$"),
        ("DE", r"^[0-9]{5}$"),
        ("FR", r"^[0-9]{5}$"),
        ("IT", r"^[0-9]{5}$"),
        ("ES", r"^[0-9]{5}$"),
        ("NL", r"^[0-9]{4} ?[A-Z]{2}$"),
        ("BE", r"^[0-9]{4}$"),
        ("AT", r"^[0-9]{4}$"),
        ("CH", r"^[0-9]{4}$"),
        ("DK", r"^[0-9]{4}$"),
        ("NO", r"^[0-9]{4}$"),
        ("SE", r"^[0-9]{3} ?[0-9]{2}$"),
        ("FI", r"^[0-9]{5}$"),
        ("PL", r"^[0-9]{2}-[0-9]{3}$"),
        ("PT", r"^[0-9]{4}-[0-9]{3}$"),
        ("AU", r"^[0-9]{4}$"),
        ("NZ", r"^[0-9]{4}$"),
        ("JP", r"^[0-9]{3}-?[0-9]{4}$"),
        ("BR", r"^[0-9]{5}-?[0-9]{3}$"),
        ("IN", r"^[0-9]{6}$"),
        ("MX", r"^[0-9]{5}$"),
        ("CZ", r"^[0-9]{3} ?[0-9]{2}$"),
        ("SK", r"^[0-9]{3} ?[0-9]{2}$"),
        ("IE", r"^[A-Z0-9]{3} ?[A-Z0-9]{4}$"),
        ("SG", r"^[0-9]{6}$"),
        ("KR", r"^[0-9]{5}$"),
        ("ZA", r"^[0-9]{4}$"),
    ]
    .into_iter()
    .map(|(k, p)| (k, re(p)))
    .collect(),
});

const NO_POSTCODE: &[&str] = &["HK", "AE", "IE"];
const NEEDS_STATE: &[&str] = &["US", "CA", "AU", "BR", "MX", "IN"];
const US_STATES: &str = "AL AK AZ AR CA CO CT DE DC FL GA HI ID IL IN IA KS KY LA ME MD MA MI MN MS MO MT NE NV NH NJ NM NY NC ND OH OK OR PA RI SC SD TN TX UT VT VA WA WV WI WY PR GU VI AS MP AA AE AP";
const CA_PROVINCES: &str = "AB BC MB NB NL NS NT NU ON PE QC SK YT";

fn rx(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

/// Normalize and check an address (an object of loosely typed fields).
pub fn validate_address(a: &Map<String, Value>, countries: Option<&[String]>) -> AddressCheck {
    let r = &*ADDRESS;
    let s = |k: &str| -> String {
        match a.get(k) {
            Some(Value::String(v)) => js::trim(&r.ws.replace_all(v, " ")).to_string(),
            _ => String::new(),
        }
    };
    let opt = |v: String| if v.is_empty() { None } else { Some(v) };
    let country = s("country").to_uppercase();
    let address = ShippingAddress {
        name: s("name"),
        line1: s("line1"),
        city: s("city"),
        postal_code: s("postalCode").to_uppercase(),
        country: country.clone(),
        company: opt(s("company")),
        line2: opt(s("line2")),
        state: opt(s("state")).map(|st| if country == "US" || country == "CA" { st.to_uppercase() } else { st }),
        phone: opt(s("phone")),
        email: opt(s("email")).map(|e| e.to_lowercase()),
    };
    let mut errors: IndexMap<String, String> = IndexMap::new();
    let mut set = |k: &str, v: &str| {
        errors.insert(k.to_string(), v.to_string());
    };
    for (f, v) in [("name", &address.name), ("line1", &address.line1), ("city", &address.city)] {
        if v.is_empty() {
            set(f, "required");
        }
    }
    let entries: [(&str, Option<&String>); 10] = [
        ("name", Some(&address.name)),
        ("line1", Some(&address.line1)),
        ("city", Some(&address.city)),
        ("postalCode", Some(&address.postal_code)),
        ("country", Some(&address.country)),
        ("company", address.company.as_ref()),
        ("line2", address.line2.as_ref()),
        ("state", address.state.as_ref()),
        ("phone", address.phone.as_ref()),
        ("email", address.email.as_ref()),
    ];
    for (f, v) in entries {
        if let Some(v) = v
            && js::utf16_len(v) > 120
        {
            set(f, "too_long");
        }
    }
    if !rx(&r.country, &country) {
        set("country", if country.is_empty() { "required" } else { "invalid" });
    } else if let Some(list) = countries
        && !list.contains(&country)
    {
        set("country", "unsupported");
    }
    if address.postal_code.is_empty() {
        if !NO_POSTCODE.contains(&country.as_str()) {
            set("postalCode", "required");
        }
    } else if let Some((_, p)) = r.postcode.iter().find(|(k, _)| *k == country)
        && !rx(p, &address.postal_code)
    {
        set("postalCode", "invalid");
    }
    let state = address.state.as_deref();
    if NEEDS_STATE.contains(&country.as_str()) && state.is_none() {
        set("state", "required");
    } else if (country == "US" && state.is_some_and(|s| !US_STATES.split(' ').any(|x| x == s)))
        || (country == "CA" && state.is_some_and(|s| !CA_PROVINCES.split(' ').any(|x| x == s)))
    {
        set("state", "invalid");
    }
    if address.phone.as_deref().is_some_and(|p| !rx(&r.phone, p)) {
        set("phone", "invalid");
    }
    if address.email.as_deref().is_some_and(|e| !rx(&r.email, e)) {
        set("email", "invalid");
    }
    AddressCheck { ok: errors.is_empty(), address, errors }
}

// ---------------------------------------------------------------- preflight

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PreflightFix {
    #[serde(rename = "resize-design")]
    ResizeDesign { w: f64, h: f64, unit: Unit },
    #[serde(rename = "extend-to-bleed", rename_all = "camelCase")]
    ExtendToBleed { element_id: String, xf: Xf },
    #[serde(rename = "thicken-line", rename_all = "camelCase")]
    ThickenLine { element_id: String, width: f64 },
    #[serde(rename = "add-page")]
    AddPage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightIssue {
    /// 'size_mismatch' | 'size_scaled' | 'page_count_over' | 'page_count_under' | 'low_dpi' | 'dpi_below_recommended'
    /// | 'text_outside_safe' | 'bleed_missing' | 'out_of_gamut' | 'thin_line' | 'font_missing' | 'not_printable'
    pub code: String,
    /// 'error' | 'warning' | 'info'
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// Values for the message (numbers are already rounded).
    pub vars: Map<String, Value>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<PreflightFix>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightTarget {
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
    pub bleed_mm: f64,
    pub safe_mm: f64,
    pub rotatable: bool,
    pub min_dpi: f64,
    pub good_dpi: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreflightResult {
    pub ok: bool,
    pub issues: Vec<PreflightIssue>,
    /// Print size ÷ design size (1 when the design already has the product size).
    pub scale: f64,
    pub trim: Trim,
    pub counts: SeverityCounts,
}

pub fn preflight_target_for(p: &PrintProduct, size_id: &str, options: &IndexMap<String, String>) -> Result<PreflightTarget, PrintError> {
    let s = p.sizes.iter().find(|x| x.id == size_id).unwrap_or(&p.sizes[0]);
    Ok(PreflightTarget {
        w: s.w,
        h: s.h,
        unit: s.unit.clone(),
        bleed_mm: p.bleed_mm,
        safe_mm: p.safe_mm,
        rotatable: p.rotatable,
        min_dpi: p.min_dpi,
        good_dpi: p.good_dpi,
        pages: Some(expected_pages(p, options)?),
    })
}

/// Generic print target for a design on its own: its own size, 3 mm bleed, 5 mm safe zone.
pub fn standalone_print_target(doc: &DesignDoc) -> PreflightTarget {
    PreflightTarget {
        w: doc.meta.dims.w,
        h: doc.meta.dims.h,
        unit: doc.meta.dims.unit.clone(),
        bleed_mm: 3.0,
        safe_mm: 5.0,
        rotatable: false,
        min_dpi: 150.0,
        good_dpi: 300.0,
        pages: None,
    }
}

/// Rough check: does this sRGB colour fall outside a typical coated CMYK gamut?
pub fn cmyk_gamut_risk(c: &ColorValue) -> bool {
    if matches!(c, ColorValue::Cmyk(_) | ColorValue::Spot(_)) {
        return false;
    }
    let x = to_rgba(c);
    let (r, g, b, a) = (x.r, x.g, x.b, x.a);
    if a < 0.05 {
        return false;
    }
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
    if sat < 0.82 || v < 0.72 {
        return false;
    }
    let d = max - min;
    let mut hue = 0.0;
    if d > 0.0 {
        hue = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
    }
    if hue < 0.0 {
        hue += 360.0;
    }
    let safe = hue < 12.0 || (45.0..=70.0).contains(&hue) || (175.0..=200.0).contains(&hue) || (300.0..=345.0).contains(&hue) || hue > 350.0;
    if safe {
        return false;
    }
    if (12.0..45.0).contains(&hue) {
        return sat > 0.92 && v > 0.95;
    }
    true
}

fn paint_colors(p: Option<&Paint>) -> Vec<&ColorValue> {
    match p {
        Some(Paint::Solid(s)) => vec![&s.color],
        Some(Paint::Linear(l)) => l.stops.iter().map(|s| &s.color).collect(),
        Some(Paint::Radial(r)) => r.stops.iter().map(|s| &s.color).collect(),
        _ => vec![],
    }
}

fn element_colors(el: &Element) -> Vec<&ColorValue> {
    match el {
        Element::Shape(e) => e
            .fills
            .iter()
            .flat_map(|f| paint_colors(Some(f)))
            .chain(e.strokes.iter().flat_map(|s| paint_colors(Some(&s.paint))))
            .collect(),
        Element::Line(e) => paint_colors(Some(&e.stroke.paint)),
        Element::Text(e) => std::iter::once(Some(&e.body.default_run.fill))
            .chain(e.body.paragraphs.iter().flat_map(|p| p.runs.iter().map(|r| r.style.fill.as_ref())))
            .flat_map(paint_colors)
            .collect(),
        Element::Qr(e) => vec![&e.fg, &e.bg],
        Element::Draw(e) => vec![&e.color],
        Element::Frame(e) => {
            if e.content.is_some() {
                vec![]
            } else {
                paint_colors(Some(&e.placeholder_fill))
            }
        }
        _ => vec![],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Axis-aligned bounds of a (possibly rotated) element box, page px.
pub fn element_bounds(xf: &Xf) -> Bounds {
    let cx = xf.x + xf.w / 2.0;
    let cy = xf.y + xf.h / 2.0;
    let r = (xf.rot * std::f64::consts::PI) / 180.0;
    let hw = (r.cos().abs() * xf.w + r.sin().abs() * xf.h) / 2.0;
    let hh = (r.sin().abs() * xf.w + r.cos().abs() * xf.h) / 2.0;
    Bounds { x0: cx - hw, y0: cy - hh, x1: cx + hw, y1: cy + hh }
}

struct ImageSource {
    src_w: f64,
    src_h: f64,
    box_w: f64,
    box_h: f64,
}

fn image_sources(doc: &DesignDoc, el: &Element) -> Vec<ImageSource> {
    let mut out = Vec::new();
    let mut add = |id: &str, crop: Option<(f64, f64)>, box_w: f64, box_h: f64| {
        let Some(a) = doc.assets.get(id) else { return };
        if a.kind.as_str() == "svg" || !(a.w > 0.0 && a.h > 0.0) {
            return;
        }
        let (cw, ch) = crop.unwrap_or((1.0, 1.0));
        out.push(ImageSource { src_w: a.w * cw, src_h: a.h * ch, box_w, box_h });
    };
    match el {
        Element::Image(e) => add(&e.asset, Some((e.crop.w, e.crop.h)), e.base.xf.w, e.base.xf.h),
        Element::Frame(e) => {
            if let Some(c) = &e.content
                && c.video.is_none()
            {
                add(&c.asset, Some((c.crop.w, c.crop.h)), e.base.xf.w, e.base.xf.h);
            }
        }
        Element::Grid(e) => {
            for c in &e.cells {
                let Some(asset) = c.asset.as_deref().filter(|a| !a.is_empty()) else { continue };
                if c.video.is_some() {
                    continue;
                }
                let bx = grid_cell_box(e.gap, &e.base.xf, &Rect::from(&c.rect));
                add(asset, c.crop.as_ref().map(|r| (r.w, r.h)), bx.w, bx.h);
            }
        }
        Element::Shape(e) => {
            for f in &e.fills {
                if let Paint::Image(i) = f {
                    add(&i.asset, None, e.base.xf.w, e.base.xf.h);
                }
            }
        }
        _ => {}
    }
    out
}

fn fonts_of(el: &TextEl) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |f: &str| {
        if !out.iter().any(|x| x == f) {
            out.push(f.to_string());
        }
    };
    push(&el.body.default_run.family);
    for p in &el.body.paragraphs {
        for r in &p.runs {
            if let Some(f) = r.style.family.as_deref().filter(|f| !f.is_empty()) {
                push(f);
            }
        }
    }
    out
}

/// `px` in a display unit, rounded (`pxToDisplay`).
fn px_to_display(px: f64, unit: &Unit) -> String {
    let per = match unit {
        Unit::Px => 1.0,
        Unit::Mm => MM,
        Unit::Cm => MM * 10.0,
        Unit::In => IN,
        Unit::Pt => 96.0 / 72.0,
        Unit::Other(_) => f64::NAN,
    };
    let v = px / per;
    if *unit == Unit::Px { js::num_to_string(js::round(v)) } else { js::num_to_string(js::round(v * 10.0) / 10.0) }
}

/// JS `n.toString(16)` for an integer-valued number.
fn int_hex(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    let i = n as i64;
    if i < 0 { format!("-{:x}", -i) } else { format!("{i:x}") }
}

/// Print preflight (size, page count, image resolution, safe zone, bleed, gamut, hairlines, fonts, interactivity).
/// `available_fonts` = `opts.availableFonts.has`. Elements of unknown types are skipped (TS would throw on them).
pub fn preflight_print(doc: &DesignDoc, target: &PreflightTarget, available_fonts: Option<&dyn Fn(&str) -> bool>) -> PreflightResult {
    preflight_print_with(doc, target, available_fonts, None)
}

/// `opts.outOfGamut`: `Some(verdict)` overrides the heuristic for a colour, `None` keeps it.
pub type OutOfGamut<'a> = &'a dyn Fn(&ColorValue) -> Option<bool>;

/// [`preflight_print`] with `opts.outOfGamut`: an out-of-gamut test that replaces the `cmyk_gamut_risk` heuristic
/// (e.g. a colour-managed check against the chosen output profile); `None` from it falls back to the heuristic.
pub fn preflight_print_with(
    doc: &DesignDoc,
    target: &PreflightTarget,
    available_fonts: Option<&dyn Fn(&str) -> bool>,
    out_of_gamut: Option<OutOfGamut<'_>>,
) -> PreflightResult {
    let mut issues: Vec<PreflightIssue> = Vec::new();
    let (dw, dh) = (doc.meta.dims.w, doc.meta.dims.h);
    let (mut tw, mut th) = (target.w, target.h);
    if target.rotatable && (dw > dh) != (tw > th) && (dw - dh).abs() > 1.0 {
        std::mem::swap(&mut tw, &mut th);
    }
    let tol = MM;
    let exact = (dw - tw).abs() <= tol && (dh - th).abs() <= tol;
    let same_aspect = (dw / dh - tw / th).abs() / (tw / th) < 0.01;
    let scale = if exact { 1.0 } else if same_aspect { tw / dw } else { (tw / dw).min(th / dh) };
    let size = |w: f64, h: f64| format!("{} × {} {}", px_to_display(w, &target.unit), px_to_display(h, &target.unit), target.unit);
    let issue = |code: &str, severity: &str, vars: Value, message: String| PreflightIssue {
        code: code.into(),
        severity: severity.into(),
        page_id: None,
        page_index: None,
        element_id: None,
        vars: match vars {
            Value::Object(m) => m,
            _ => Map::new(),
        },
        message,
        fix: None,
    };
    if !exact {
        let fix = PreflightFix::ResizeDesign { w: tw, h: th, unit: target.unit.clone() };
        let vars = json!({ "design": size(dw, dh), "product": size(tw, th) });
        let mut i = if same_aspect {
            issue("size_scaled", "warning", vars, format!("The design ({}) will be scaled to the product size ({}).", size(dw, dh), size(tw, th)))
        } else {
            issue(
                "size_mismatch",
                "error",
                vars,
                format!("The design is {} but the product is {}. Resize the design to the product.", size(dw, dh), size(tw, th)),
            )
        };
        i.fix = Some(fix);
        issues.push(i);
    }

    let pages: Vec<_> = sorted_pages(&doc.pages).into_iter().filter(|p| p.hidden != Some(true)).collect();
    if let Some(expected) = target.pages {
        let n = pages.len() as f64;
        let e = js::num_to_string(expected);
        if n > expected {
            issues.push(issue(
                "page_count_over",
                "error",
                json!({ "pages": pages.len(), "expected": expected }),
                format!("The design has {} pages but this product prints {e}.", pages.len()),
            ));
        } else if n < expected {
            let mut i = issue(
                "page_count_under",
                "warning",
                json!({ "pages": pages.len(), "expected": expected }),
                format!("This product prints {e} sides; the design has {}, so the back will be blank.", pages.len()),
            );
            i.fix = Some(PreflightFix::AddPage);
            issues.push(i);
        }
    }

    let bleed = (target.bleed_mm * MM) / scale;
    let safe = (target.safe_mm * MM) / scale;
    let hairline = (0.25 * (96.0 / 72.0)) / scale;
    let (w_, h_) = (dw, dh);
    let mut gamut_seen: HashSet<String> = HashSet::new();
    let mut font_seen: HashSet<String> = HashSet::new();
    let bleed_s = js::num_to_string(target.bleed_mm);
    let safe_s = js::num_to_string(target.safe_mm);

    let mut gamut = |issues: &mut Vec<PreflightIssue>, c: &ColorValue, page_id: &str, page_index: usize, element_id: Option<&str>| {
        if !out_of_gamut.and_then(|f| f(c)).unwrap_or_else(|| cmyk_gamut_risk(c)) {
            return;
        }
        let x = to_rgba(c);
        let hex = format!(
            "#{}",
            [x.r, x.g, x.b].iter().map(|v| format!("{:0>2}", int_hex(js::round(v * 255.0)))).collect::<String>()
        );
        if !gamut_seen.insert(hex.clone()) {
            return;
        }
        let mut i = issue("out_of_gamut", "info", json!({ "color": hex }), format!("{hex} is brighter than print inks can reproduce and will print duller."));
        i.page_id = Some(page_id.into());
        i.page_index = Some(page_index);
        i.element_id = element_id.map(String::from);
        issues.push(i);
    };

    for (page_index, page) in pages.iter().enumerate() {
        for bg in &page.background {
            for c in paint_colors(Some(bg)) {
                gamut(&mut issues, c, &page.id, page_index, None);
            }
        }
        for el in page_elements(&doc.elements, &page.id) {
            let Some(base) = el.base() else { continue };
            if base.visible == Some(false) || base.deleted.is_some() {
                continue;
            }
            let at = |mut i: PreflightIssue| {
                i.page_id = Some(page.id.clone());
                i.page_index = Some(page_index);
                i.element_id = Some(base.id.clone());
                i
            };
            let name = base.name.clone().unwrap_or_else(|| el.type_name().to_string());
            let frame_video = matches!(el, Element::Frame(f) if f.content.as_ref().is_some_and(|c| c.video.is_some()));
            if matches!(el, Element::Video(_) | Element::Embed(_) | Element::Form(_)) || frame_video {
                issues.push(at(issue(
                    "not_printable",
                    "warning",
                    json!({ "name": name }),
                    format!("“{name}” is interactive or moving and prints as a still image."),
                )));
            }
            for src in image_sources(doc, el) {
                let dpi = (src.src_w / ((src.box_w * scale) / 96.0)).min(src.src_h / ((src.box_h * scale) / 96.0));
                let dpi = {
                    // Math.min propagates NaN; f64::min does not.
                    let a = src.src_w / ((src.box_w * scale) / 96.0);
                    let b = src.src_h / ((src.box_h * scale) / 96.0);
                    if a.is_nan() || b.is_nan() { f64::NAN } else { dpi }
                };
                if !dpi.is_finite() {
                    continue;
                }
                let rd = js::round(dpi);
                if dpi < target.min_dpi {
                    issues.push(at(issue(
                        "low_dpi",
                        "warning",
                        json!({ "dpi": rd, "min": target.min_dpi, "name": name }),
                        format!(
                            "“{name}” prints at {} dpi (below {}); it will look blurry. Use a larger image or make it smaller.",
                            js::num_to_string(rd),
                            js::num_to_string(target.min_dpi)
                        ),
                    )));
                } else if dpi < target.good_dpi {
                    issues.push(at(issue(
                        "dpi_below_recommended",
                        "info",
                        json!({ "dpi": rd, "good": target.good_dpi, "name": name }),
                        format!(
                            "“{name}” prints at {} dpi; {} dpi is recommended for sharp detail.",
                            js::num_to_string(rd),
                            js::num_to_string(target.good_dpi)
                        ),
                    )));
                }
            }
            let b = element_bounds(&base.xf);
            if let Element::Text(t) = el {
                let has_text = t.body.paragraphs.iter().any(|p| p.runs.iter().any(|r| !js::trim(&r.text).is_empty()));
                if target.safe_mm > 0.0
                    && has_text
                    && (b.x0 < safe - 0.5 || b.y0 < safe - 0.5 || b.x1 > w_ - safe + 0.5 || b.y1 > h_ - safe + 0.5)
                {
                    issues.push(at(issue(
                        "text_outside_safe",
                        "warning",
                        json!({ "name": name, "safe": target.safe_mm }),
                        format!("Text “{name}” is within {safe_s} mm of the edge and may be trimmed."),
                    )));
                }
                if let Some(has) = available_fonts {
                    for f in fonts_of(t) {
                        if !has(&f) && !font_seen.contains(&f) {
                            font_seen.insert(f.clone());
                            issues.push(at(issue(
                                "font_missing",
                                "warning",
                                json!({ "font": f }),
                                format!("The font “{f}” isn’t available to the print renderer and will be replaced."),
                            )));
                        }
                    }
                }
            }
            if target.bleed_mm > 0.0 && base.parent_id == page.id && !matches!(el, Element::Text(_) | Element::Line(_) | Element::Group(_)) {
                let (tl, tt) = (b.x0 <= 0.5 && b.x0 > -bleed + 0.5, b.y0 <= 0.5 && b.y0 > -bleed + 0.5);
                let (tr, tb) = (b.x1 >= w_ - 0.5 && b.x1 < w_ + bleed - 0.5, b.y1 >= h_ - 0.5 && b.y1 < h_ + bleed - 0.5);
                let span_x = b.x1.min(w_) - b.x0.max(0.0);
                let span_y = b.y1.min(h_) - b.y0.max(0.0);
                let mut sides: Vec<char> = Vec::new();
                for (k, t) in [('l', tl), ('r', tr)] {
                    if t && span_y > h_ * 0.2 {
                        sides.push(k);
                    }
                }
                for (k, t) in [('t', tt), ('b', tb)] {
                    if t && span_x > w_ * 0.2 {
                        sides.push(k);
                    }
                }
                if !sides.is_empty() {
                    let mut xf = base.xf.clone();
                    if base.xf.rot == 0.0 {
                        if sides.contains(&'l') {
                            xf.w += xf.x + bleed;
                            xf.x = -bleed;
                        }
                        if sides.contains(&'t') {
                            xf.h += xf.y + bleed;
                            xf.y = -bleed;
                        }
                        if sides.contains(&'r') {
                            xf.w = w_ + bleed - xf.x;
                        }
                        if sides.contains(&'b') {
                            xf.h = h_ + bleed - xf.y;
                        }
                    }
                    let mut i = at(issue(
                        "bleed_missing",
                        "warning",
                        json!({ "name": name, "bleed": target.bleed_mm }),
                        format!("“{name}” stops at the trim edge; extend it {bleed_s} mm past the edge so no white line shows after cutting."),
                    ));
                    if base.xf.rot == 0.0 {
                        i.fix = Some(PreflightFix::ExtendToBleed { element_id: base.id.clone(), xf });
                    }
                    issues.push(i);
                }
            }
            let stroke_w = match el {
                Element::Line(l) => Some(l.stroke.width),
                Element::Shape(s) if !s.strokes.is_empty() => Some(s.strokes.iter().map(|x| x.width).fold(f64::INFINITY, f64::min)),
                _ => None,
            };
            if let Some(sw) = stroke_w
                && sw > 0.0
                && sw < hairline
            {
                let width = js::round(sw * scale * 0.75 * 100.0) / 100.0;
                let mut i = at(issue(
                    "thin_line",
                    "warning",
                    json!({ "name": name, "width": width }),
                    format!("“{name}” has a {} pt line; lines under 0.25 pt may not print.", js::num_to_string(width)),
                ));
                i.fix = Some(PreflightFix::ThickenLine { element_id: base.id.clone(), width: (hairline * 100.0).ceil() / 100.0 });
                issues.push(i);
            }
            for c in element_colors(el) {
                gamut(&mut issues, c, &page.id, page_index, Some(&base.id));
            }
        }
    }

    let mut counts = SeverityCounts::default();
    for i in &issues {
        match i.severity.as_str() {
            "error" => counts.error += 1,
            "warning" => counts.warning += 1,
            _ => counts.info += 1,
        }
    }
    PreflightResult { ok: counts.error == 0, issues, scale, trim: Trim { w: tw, h: th }, counts }
}

/// Bleed in design px for a product (for the export job).
pub fn bleed_px_for(bleed_mm: f64) -> f64 {
    bleed_mm * MM
}

/// Unique colours that can fall outside a CMYK gamut (sRGB / Display-P3, not transparent), in the order preflight
/// meets them: page backgrounds, then visible elements. Capped at `max` (for a colour-managed gamut check).
/// TS `printColors(doc, max = 512)`.
pub fn print_colors(doc: &DesignDoc, max: usize) -> Vec<ColorValue> {
    let mut out: Vec<ColorValue> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut add = |c: &ColorValue| {
        if out.len() >= max || matches!(c, ColorValue::Cmyk(_) | ColorValue::Spot(_)) || to_rgba(c).a < 0.05 {
            return;
        }
        if seen.insert(print_color_key(c)) {
            out.push(c.clone());
        }
    };
    for page in sorted_pages(&doc.pages).into_iter().filter(|p| p.hidden != Some(true)) {
        for bg in &page.background {
            paint_colors(Some(bg)).into_iter().for_each(&mut add);
        }
        for el in page_elements(&doc.elements, &page.id) {
            let Some(base) = el.base() else { continue };
            if base.visible == Some(false) || el.is_deleted() {
                continue;
            }
            element_colors(el).into_iter().for_each(&mut add);
        }
    }
    out
}

/// Identity of a colour for gamut lookups (space + rounded components, alpha ignored). TS `printColorKey`.
pub fn print_color_key(c: &ColorValue) -> String {
    let x = to_rgba(c);
    let parts: Vec<String> = [x.r, x.g, x.b].iter().map(|v| js::num_to_string(js::round(v * 1000.0))).collect();
    format!("{}:{}", c.space(), parts.join(","))
}
