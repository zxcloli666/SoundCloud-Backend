pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for i in 0..n {
        dot += (a[i] as f64) * (b[i] as f64);
        na += (a[i] as f64) * (a[i] as f64);
        nb += (b[i] as f64) * (b[i] as f64);
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom > 0.0 {
        (dot / denom) as f32
    } else {
        0.0
    }
}

pub fn normalize(v: &mut [f32]) {
    let norm = v
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm > 0.0 {
        for x in v {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-5
    }

    #[test]
    fn the_same_direction_is_one_and_the_opposite_is_minus_one() {
        assert!(close(cosine(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]), 1.0));
        assert!(close(cosine(&[1.0, 0.0], &[-1.0, 0.0]), -1.0));
        assert!(close(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0));
    }

    #[test]
    fn a_vector_of_zeros_reads_as_no_similarity_and_not_as_a_nan() {
        let against_zero = cosine(&[0.0, 0.0, 0.0], &[1.0, 2.0, 3.0]);
        assert_eq!(against_zero, 0.0);
        assert!(!cosine(&[0.0], &[0.0]).is_nan());
    }

    #[test]
    fn vectors_of_different_length_compare_on_the_shared_prefix() {
        assert!(close(cosine(&[1.0, 0.0, 99.0], &[1.0, 0.0]), 1.0));
        assert_eq!(cosine(&[], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn normalizing_makes_the_length_one() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        assert!(close(v[0], 0.6));
        assert!(close(v[1], 0.8));
    }

    #[test]
    fn normalizing_a_vector_of_zeros_leaves_it_alone_instead_of_making_nans() {
        let mut v = vec![0.0, 0.0, 0.0];
        normalize(&mut v);
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
        assert!(v.iter().all(|x| !x.is_nan()));
    }
}
