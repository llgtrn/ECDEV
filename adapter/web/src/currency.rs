//! ISO 4217 minor units, kept apart from the locked price-parser port in price.rs.
/// ISO 4217 minor-unit digits for the active currency codes, `None` for anything else.
/// Only a stated code is converted; a code outside the table, a fund or precious-metal code, and
/// a symbol never are. The table replaced a four-currency match that left a price unknown on
/// every page that stated AUD, NOK, CAD, CHF and the rest: live, 46 of 458 JSON-LD offers on 32
/// retail hosts (Sonos NO, Uniqlo AU).
pub fn minor_unit_digits(currency: &str) -> Option<usize> {
    const ZERO: &[&str] = &[
        "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "VND", "VUV",
        "XAF", "XOF", "XPF",
    ];
    const THREE: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];
    const TWO: &[&str] = &[
        "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
        "BGN", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF", "CHF",
        "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR",
        "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF",
        "IDR", "ILS", "INR", "IRR", "JMD", "KES", "KGS", "KHR", "KPW", "KYD", "KZT", "LAK", "LBP",
        "LKR", "LRD", "LSL", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR",
        "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "PAB", "PEN", "PGK",
        "PHP", "PKR", "PLN", "QAR", "RON", "RSD", "RUB", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD",
        "SHP", "SLE", "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TOP",
        "TRY", "TTD", "TWD", "TZS", "UAH", "USD", "UYU", "UZS", "VES", "WST", "XCD", "YER", "ZAR",
        "ZMW", "ZWG",
    ];
    if ZERO.contains(&currency) {
        Some(0)
    } else if THREE.contains(&currency) {
        Some(3)
    } else if TWO.contains(&currency) {
        Some(2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minor_unit_digits_follow_iso_4217_and_refuse_unknown_codes() {
        for (c, d) in [
            ("JPY", 0),
            ("KRW", 0),
            ("USD", 2),
            ("AUD", 2),
            ("NOK", 2),
            ("CHF", 2),
            ("KWD", 3),
            ("BHD", 3),
        ] {
            assert_eq!(minor_unit_digits(c), Some(d), "{c}");
        }
        for c in ["", "usd", "XYZ", "XAU", "XXX", "US", "$", "USDD", "CLF"] {
            assert_eq!(minor_unit_digits(c), None, "{c}");
        }
    }
}
