use super::{Deserialize, Serialize};

/// Explicit conditional table-style flags (`w:cnfStyle`). `val` is the twelve-bit OOXML mask;
/// named flags remain optional so explicit false values and inherited flags stay distinct.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CnfStyle {
    pub val: Option<String>,
    pub first_row: Option<bool>,
    pub last_row: Option<bool>,
    pub first_column: Option<bool>,
    pub last_column: Option<bool>,
    pub odd_v_band: Option<bool>,
    pub even_v_band: Option<bool>,
    pub odd_h_band: Option<bool>,
    pub even_h_band: Option<bool>,
    pub first_row_first_column: Option<bool>,
    pub first_row_last_column: Option<bool>,
    pub last_row_first_column: Option<bool>,
    pub last_row_last_column: Option<bool>,
}

impl CnfStyle {
    /// Keep only schema-sized masks. Malformed input is ignored rather than allocated or copied.
    pub fn bounded_val(v: &str) -> Option<String> {
        (v.len() == 12 && v.bytes().all(|b| matches!(b, b'0' | b'1'))).then(|| v.to_owned())
    }

    pub fn overlay(&mut self, patch: &Self) {
        macro_rules! fields {
            ($($field:ident),+ $(,)?) => {$(if patch.$field.is_some() { self.$field = patch.$field.clone(); })+};
        }
        fields!(
            val,
            first_row,
            last_row,
            first_column,
            last_column,
            odd_v_band,
            even_v_band,
            odd_h_band,
            even_h_band,
            first_row_first_column,
            first_row_last_column,
            last_row_first_column,
            last_row_last_column
        );
    }

    pub fn minus(mut self, base: &Self) -> Self {
        macro_rules! fields {
            ($($field:ident),+ $(,)?) => {$(if self.$field == base.$field { self.$field = None; })+};
        }
        fields!(
            val,
            first_row,
            last_row,
            first_column,
            last_column,
            odd_v_band,
            even_v_band,
            odd_h_band,
            even_h_band,
            first_row_first_column,
            first_row_last_column,
            last_row_first_column,
            last_row_last_column
        );
        self
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::CnfStyle;

    #[test]
    fn conditional_mask_accepts_only_twelve_bits() {
        assert_eq!(CnfStyle::bounded_val("100000000000").as_deref(), Some("100000000000"));
        assert_eq!(CnfStyle::bounded_val("10000000000"), None);
        assert_eq!(CnfStyle::bounded_val("10000000000x"), None);
        assert_eq!(CnfStyle::bounded_val(&"1".repeat(100_000)), None);
    }
}
