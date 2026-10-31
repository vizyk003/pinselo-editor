// SPDX-License-Identifier: AGPL-3.0-only
//! Function coverage: every function HyperFormula 3.4.0 registers (`HyperFormula.getRegisteredFunctionNames('enGB')`)
//! plus the CONCAT plugin @oc/sheet adds is implemented, and evaluates to something other than #NAME?.

use oc_sheet::format::CellValue;
use oc_sheet::formula::functions::is_implemented;
use oc_sheet::formula::workbook::{FixedHost, Workbook};

/// HyperFormula 3.4.0 `getRegisteredFunctionNames('enGB')` + CONCAT (424 names).
const HF_FUNCTIONS: &[&str] = &[
    "ABS", "ACOS", "ACOSH", "ACOT", "ACOTH", "ADDRESS", "AND", "ARABIC", "ARRAYFORMULA", "ARRAY_CONSTRAIN", "ASIN",
    "ASINH", "ATAN", "ATAN2", "ATANH", "AVEDEV", "AVERAGE", "AVERAGEA", "AVERAGEIF", "BASE", "BESSELI", "BESSELJ",
    "BESSELK", "BESSELY", "BETA.DIST", "BETA.INV", "BETADIST", "BETAINV", "BIN2DEC", "BIN2HEX", "BIN2OCT",
    "BINOM.DIST", "BINOM.INV", "BINOMDIST", "BITAND", "BITLSHIFT", "BITOR", "BITRSHIFT", "BITXOR", "CEILING",
    "CEILING.MATH", "CEILING.PRECISE", "CHAR", "CHIDIST", "CHIDISTRT", "CHIINV", "CHIINVRT", "CHISQ.DIST",
    "CHISQ.DIST.RT", "CHISQ.INV", "CHISQ.INV.RT", "CHISQ.TEST", "CHITEST", "CHOOSE", "CLEAN", "CODE", "COLUMN",
    "COLUMNS", "COMBIN", "COMBINA", "COMPLEX", "CONCAT", "CONCATENATE", "CONFIDENCE", "CONFIDENCE.NORM",
    "CONFIDENCE.T", "CORREL", "COS", "COSH", "COT", "COTH", "COUNT", "COUNTA", "COUNTBLANK", "COUNTIF", "COUNTIFS",
    "COUNTUNIQUE", "COVAR", "COVARIANCE.P", "COVARIANCE.S", "COVARIANCEP", "COVARIANCES", "CRITBINOM", "CSC", "CSCH",
    "CUMIPMT", "CUMPRINC", "DATE", "DATEDIF", "DATEVALUE", "DAVERAGE", "DAY", "DAYS", "DAYS360", "DB", "DCOUNT",
    "DCOUNTA", "DDB", "DEC2BIN", "DEC2HEX", "DEC2OCT", "DECIMAL", "DEGREES", "DELTA", "DEVSQ", "DGET", "DMAX",
    "DMIN", "DOLLARDE", "DOLLARFR", "DPRODUCT", "DSTDEV", "DSTDEVP", "DSUM", "DVAR", "DVARP", "EDATE", "EFFECT",
    "EOMONTH", "ERF", "ERFC", "EVEN", "EXACT", "EXP", "EXPON.DIST", "EXPONDIST", "F.DIST", "F.DIST.RT", "F.INV",
    "F.INV.RT", "F.TEST", "FACT", "FACTDOUBLE", "FALSE", "FDIST", "FDISTRT", "FILTER", "FIND", "FINV", "FINVRT",
    "FISHER", "FISHERINV", "FLOOR", "FLOOR.MATH", "FLOOR.PRECISE", "FORMULATEXT", "FTEST", "FV", "FVSCHEDULE",
    "GAMMA", "GAMMA.DIST", "GAMMA.INV", "GAMMADIST", "GAMMAINV", "GAMMALN", "GAMMALN.PRECISE", "GAUSS", "GCD",
    "GEOMEAN", "HARMEAN", "HEX2BIN", "HEX2DEC", "HEX2OCT", "HF.ADD", "HF.CONCAT", "HF.DIVIDE", "HF.EQ", "HF.GT",
    "HF.GTE", "HF.LT", "HF.LTE", "HF.MINUS", "HF.MULTIPLY", "HF.NE", "HF.POW", "HF.UMINUS", "HF.UNARY_PERCENT",
    "HF.UPLUS", "HLOOKUP", "HOUR", "HSTACK", "HYPERLINK", "HYPGEOM.DIST", "HYPGEOMDIST", "IF", "IFERROR", "IFNA",
    "IFS", "IMABS", "IMAGINARY", "IMARGUMENT", "IMCONJUGATE", "IMCOS", "IMCOSH", "IMCOT", "IMCSC", "IMCSCH", "IMDIV",
    "IMEXP", "IMLN", "IMLOG10", "IMLOG2", "IMPOWER", "IMPRODUCT", "IMREAL", "IMSEC", "IMSECH", "IMSIN", "IMSINH",
    "IMSQRT", "IMSUB", "IMSUM", "IMTAN", "INDEX", "INT", "INTERVAL", "IPMT", "IRR", "ISBINARY", "ISBLANK", "ISERR",
    "ISERROR", "ISEVEN", "ISFORMULA", "ISLOGICAL", "ISNA", "ISNONTEXT", "ISNUMBER", "ISO.CEILING", "ISODD",
    "ISOWEEKNUM", "ISPMT", "ISREF", "ISTEXT", "LARGE", "LCM", "LEFT", "LEN", "LN", "LOG", "LOG10", "LOGINV",
    "LOGNORM.DIST", "LOGNORM.INV", "LOGNORMDIST", "LOGNORMINV", "LOWER", "MATCH", "MAX", "MAXA", "MAXIFS", "MAXPOOL",
    "MEDIAN", "MEDIANPOOL", "MID", "MIN", "MINA", "MINIFS", "MINUTE", "MIRR", "MMULT", "MOD", "MONTH", "MROUND",
    "MULTINOMIAL", "N", "NA", "NEGBINOM.DIST", "NEGBINOMDIST", "NETWORKDAYS", "NETWORKDAYS.INTL", "NOMINAL",
    "NORM.DIST", "NORM.INV", "NORM.S.DIST", "NORM.S.INV", "NORMDIST", "NORMINV", "NORMSDIST", "NORMSINV", "NOT",
    "NOW", "NPER", "NPV", "OCT2BIN", "OCT2DEC", "OCT2HEX", "ODD", "OFFSET", "OR", "PDURATION", "PEARSON",
    "PERCENTILE", "PERCENTILE.EXC", "PERCENTILE.INC", "PHI", "PI", "PMT", "POISSON", "POISSON.DIST", "POISSONDIST",
    "POWER", "PPMT", "PRODUCT", "PROPER", "PV", "QUARTILE", "QUARTILE.EXC", "QUARTILE.INC", "QUOTIENT", "RADIANS",
    "RAND", "RANDBETWEEN", "RATE", "REPLACE", "REPT", "RIGHT", "ROMAN", "ROUND", "ROUNDDOWN", "ROUNDUP", "ROW",
    "ROWS", "RRI", "RSQ", "SEARCH", "SEC", "SECH", "SECOND", "SEQUENCE", "SERIESSUM", "SHEET", "SHEETS", "SIGN",
    "SIN", "SINH", "SKEW", "SKEW.P", "SKEWP", "SLN", "SLOPE", "SMALL", "SORT", "SPLIT", "SQRT", "SQRTPI",
    "STANDARDIZE", "STDEV", "STDEV.P", "STDEV.S", "STDEVA", "STDEVP", "STDEVPA", "STDEVS", "STEYX", "SUBSTITUTE",
    "SUBTOTAL", "SUM", "SUMIF", "SUMIFS", "SUMPRODUCT", "SUMSQ", "SUMX2MY2", "SUMX2PY2", "SUMXMY2", "SWITCH", "SYD",
    "T", "T.DIST", "T.DIST.2T", "T.DIST.RT", "T.INV", "T.INV.2T", "T.TEST", "TAN", "TANH", "TBILLEQ", "TBILLPRICE",
    "TBILLYIELD", "TDIST", "TDIST2T", "TDISTRT", "TEXT", "TEXTJOIN", "TIME", "TIMEVALUE", "TINV", "TINV2T", "TODAY",
    "TRANSPOSE", "TRIM", "TRUE", "TRUNC", "TTEST", "UNICHAR", "UNICODE", "UNIQUE", "UPPER", "VALUE", "VAR", "VAR.P",
    "VAR.S", "VARA", "VARP", "VARPA", "VARS", "VERSION", "VLOOKUP", "VSTACK", "WEEKDAY", "WEEKNUM", "WEIBULL",
    "WEIBULL.DIST", "WEIBULLDIST", "WORKDAY", "WORKDAY.INTL", "XIRR", "XLOOKUP", "XNPV", "XOR", "YEAR", "YEARFRAC",
    "Z.TEST", "ZTEST",
];

#[test]
fn every_hyperformula_function_is_implemented() {
    // OFFSET is rewritten into a reference by HyperFormula's parser, not dispatched as a function.
    let missing: Vec<&str> = HF_FUNCTIONS.iter().copied().filter(|n| *n != "OFFSET" && !is_implemented(n)).collect();
    assert!(missing.is_empty(), "not implemented: {missing:?}");
}

#[test]
fn no_function_name_resolves_to_name_error() {
    let mut wb = Workbook::new(Box::new(FixedHost::default()));
    let s = wb.add_sheet("Sheet1");
    for (i, n) in HF_FUNCTIONS.iter().enumerate() {
        // Zero arguments: either a value or an arity error (#N/A), never #NAME?.
        wb.set_contents(s, i as u32, 0, Some(&format!("={n}()")));
    }
    wb.recompute();
    let bad: Vec<&str> = HF_FUNCTIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| matches!(wb.value(s, *i as u32, 0), CellValue::Error(ref e) if e == "#NAME?"))
        .map(|(_, n)| *n)
        .collect();
    assert!(bad.is_empty(), "#NAME? for: {bad:?}");
}
