//! Find work item keys (e.g. `PAY-142`) mentioned in commit messages or branch names.

/// Keys in order of first appearance, without duplicates. A key is 2–10
/// uppercase letters/digits starting with a letter, a dash, and a number,
/// standing on word boundaries.
pub fn item_keys(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if boundary && b[i].is_ascii_uppercase() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit()) {
                j += 1;
            }
            let prefix = j - i;
            if (2..=10).contains(&prefix) && j < b.len() && b[j] == b'-' {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                let digits = k - j - 1;
                let end_ok = k >= b.len() || !b[k].is_ascii_alphanumeric();
                if (1..=9).contains(&digits) && end_ok && b[j + 1] != b'0' {
                    let key = &text[i..k];
                    if !out.iter().any(|x| x == key) {
                        out.push(key.to_owned());
                    }
                    i = k;
                    continue;
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_keys() {
        assert_eq!(
            item_keys("PAY-12: fix rounding (see PAY-3, CRM-7) PAY-12"),
            vec!["PAY-12", "PAY-3", "CRM-7"]
        );
        assert_eq!(item_keys("feature/PAY-142-refunds"), vec!["PAY-142"]);
    }

    #[test]
    fn ignores_lookalikes() {
        assert!(item_keys("UTF-8 and sha256").contains(&"UTF-8".to_owned()));
        assert!(item_keys("xPAY-1 PAY-01 P-1 PAY-1a").is_empty());
        assert!(item_keys("").is_empty());
    }
}
