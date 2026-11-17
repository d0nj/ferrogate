pub fn normalise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;

    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }

    out
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_case_and_whitespace() {
        assert_eq!(normalise("  Hello   WORLD \n\tfoo "), "hello world foo");
    }

    #[test]
    fn identical_vectors_score_one() {
        let a = [0.5f32, 0.5, 0.5, 0.5];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn orthogonal_vectors_score_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-5);
    }

    #[test]
    fn opposite_vectors_score_negative() {
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn empty_vectors_do_not_panic() {
        assert_eq!(cosine(&[], &[]), 0.0);
    }
}
