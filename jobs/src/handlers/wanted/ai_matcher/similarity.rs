use std::collections::{BTreeSet, HashMap};

use crate::handlers::metadata::words;

const AUTOJUNK_FROM: usize = 200;

pub fn token_set(left: &str, right: &str) -> f64 {
    let ours: BTreeSet<String> = words(left).into_iter().collect();
    let theirs: BTreeSet<String> = words(right).into_iter().collect();
    if ours.is_empty() || theirs.is_empty() {
        return 0.0;
    }
    let shared = spaced(ours.intersection(&theirs));
    let with_ours = spaced([&shared, &spaced(ours.difference(&theirs))]);
    let with_theirs = spaced([&shared, &spaced(theirs.difference(&ours))]);
    [
        (&with_ours, &with_theirs),
        (&shared, &with_ours),
        (&shared, &with_theirs),
    ]
    .into_iter()
    .filter(|(left, right)| !left.is_empty() && !right.is_empty())
    .map(|(left, right)| ratio(left, right))
    .fold(0.0, f64::max)
}

pub fn ratio(left: &str, right: &str) -> f64 {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    let length = a.len() + b.len();
    if length == 0 {
        return 1.0;
    }
    2.0 * Matcher::new(&a, &b).matched() as f64 / length as f64
}

fn spaced<'a>(parts: impl IntoIterator<Item = &'a String>) -> String {
    let parts: Vec<&str> = parts
        .into_iter()
        .map(String::as_str)
        .filter(|part| !part.is_empty())
        .collect();
    parts.join(" ")
}

struct Matcher<'a> {
    a: &'a [char],
    b: &'a [char],
    b2j: HashMap<char, Vec<usize>>,
}

impl<'a> Matcher<'a> {
    fn new(a: &'a [char], b: &'a [char]) -> Self {
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (j, element) in b.iter().enumerate() {
            b2j.entry(*element).or_default().push(j);
        }
        if b.len() >= AUTOJUNK_FROM {
            let most = b.len() / 100 + 1;
            b2j.retain(|_, places| places.len() <= most);
        }
        Self { a, b, b2j }
    }

    fn matched(&self) -> usize {
        let mut total = 0;
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, size) = self.longest(alo, ahi, blo, bhi);
            if size == 0 {
                continue;
            }
            total += size;
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + size < ahi && j + size < bhi {
                queue.push((i + size, ahi, j + size, bhi));
            }
        }
        total
    }

    fn longest(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0);
        let mut runs: HashMap<usize, usize> = HashMap::new();
        for (i, element) in self.a.iter().enumerate().take(ahi).skip(alo) {
            let mut next_runs = HashMap::new();
            for &j in self.b2j.get(element).map_or(&[][..], Vec::as_slice) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let before = j
                    .checked_sub(1)
                    .and_then(|before| runs.get(&before))
                    .copied()
                    .unwrap_or(0);
                let size = before + 1;
                next_runs.insert(j, size);
                if size > best_size {
                    (best_i, best_j, best_size) = (i + 1 - size, j + 1 - size, size);
                }
            }
            runs = next_runs;
        }
        while best_i > alo && best_j > blo && self.same(best_i - 1, best_j - 1) {
            (best_i, best_j, best_size) = (best_i - 1, best_j - 1, best_size + 1);
        }
        while best_i + best_size < ahi
            && best_j + best_size < bhi
            && self.same(best_i + best_size, best_j + best_size)
        {
            best_size += 1;
        }
        (best_i, best_j, best_size)
    }

    fn same(&self, i: usize, j: usize) -> bool {
        matches!((self.a.get(i), self.b.get(j)), (Some(left), Some(right)) if left == right)
    }
}
