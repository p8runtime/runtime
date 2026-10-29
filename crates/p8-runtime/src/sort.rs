//! zend_sort (Zend/zend_sort.c): PHP's hybrid insertion sort and quicksort,
//! ported step for step. PHP's comparisons of mixed values are not a
//! total order ("10" < "9a" < "9" < "10"), so the order a sort leaves
//! depends on which pairs it compares and when: sorting as PHP does gives
//! PHP's order (and calls a user comparator with PHP's pairs), where a
//! sort that assumes a total order may panic or differ. Not stable by
//! itself: PHP's array sorts break ties on the original position.

use std::cmp::Ordering;

/// Sort `v` with `cmp` the way zend_sort does.
pub fn zend_sort<T>(v: &mut [T], cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    let mut base = 0;
    let mut n = v.len();
    loop {
        if n <= 16 {
            insert_sort(&mut v[base..base + n], cmp);
            return;
        }
        let start = base;
        let end = base + n;
        let offset = n >> 1;
        let mut pivot = start + offset;
        if n >> 10 != 0 {
            let delta = offset >> 1;
            sort5(v, [start, start + delta, pivot, pivot + delta, end - 1], cmp);
        } else {
            sort3(v, start, pivot, end - 1, cmp);
        }
        v.swap(start + 1, pivot);
        pivot = start + 1;
        let mut i = pivot + 1;
        let mut j = end - 1;
        'partition: loop {
            while gt(v, pivot, i, cmp) {
                i += 1;
                if i == j {
                    break 'partition;
                }
            }
            j -= 1;
            if j == i {
                break 'partition;
            }
            while gt(v, j, pivot, cmp) {
                j -= 1;
                if j == i {
                    break 'partition;
                }
            }
            v.swap(i, j);
            i += 1;
            if i == j {
                break 'partition;
            }
        }
        v.swap(pivot, i - 1);
        // Recurse into the smaller side, loop on the larger.
        if (i - 1) - start < end - i {
            zend_sort(&mut v[start..i - 1], cmp);
            base = i;
            n = end - i;
        } else {
            zend_sort(&mut v[i..end], cmp);
            base = start;
            n = (i - start) - 1;
        }
    }
}

fn gt<T>(v: &[T], a: usize, b: usize, cmp: &mut impl FnMut(&T, &T) -> Ordering) -> bool {
    cmp(&v[a], &v[b]) == Ordering::Greater
}

fn sort2<T>(v: &mut [T], a: usize, b: usize, cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    if gt(v, a, b, cmp) {
        v.swap(a, b);
    }
}

fn sort3<T>(v: &mut [T], a: usize, b: usize, c: usize, cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    if !gt(v, a, b, cmp) {
        if !gt(v, b, c, cmp) {
            return;
        }
        v.swap(b, c);
        if gt(v, a, b, cmp) {
            v.swap(a, b);
        }
        return;
    }
    if !gt(v, c, b, cmp) {
        v.swap(a, c);
        return;
    }
    v.swap(a, b);
    if gt(v, b, c, cmp) {
        v.swap(b, c);
    }
}

fn sort4<T>(v: &mut [T], [a, b, c, d]: [usize; 4], cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    sort3(v, a, b, c, cmp);
    if gt(v, c, d, cmp) {
        v.swap(c, d);
        if gt(v, b, c, cmp) {
            v.swap(b, c);
            if gt(v, a, b, cmp) {
                v.swap(a, b);
            }
        }
    }
}

fn sort5<T>(v: &mut [T], [a, b, c, d, e]: [usize; 5], cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    sort4(v, [a, b, c, d], cmp);
    if gt(v, d, e, cmp) {
        v.swap(d, e);
        if gt(v, c, d, cmp) {
            v.swap(c, d);
            if gt(v, b, c, cmp) {
                v.swap(b, c);
                if gt(v, a, b, cmp) {
                    v.swap(a, b);
                }
            }
        }
    }
}

/// zend_insert_sort: up to five by networks, else insertion, the first
/// six one step at a time and the rest two steps back first.
fn insert_sort<T>(v: &mut [T], cmp: &mut impl FnMut(&T, &T) -> Ordering) {
    let n = v.len();
    match n {
        0 | 1 => {}
        2 => sort2(v, 0, 1, cmp),
        3 => sort3(v, 0, 1, 2, cmp),
        4 => sort4(v, [0, 1, 2, 3], cmp),
        5 => sort5(v, [0, 1, 2, 3, 4], cmp),
        _ => {
            let sentry = 6.min(n);
            for i in 1..sentry {
                let mut j = i - 1;
                if !gt(v, j, i, cmp) {
                    continue;
                }
                while j != 0 {
                    j -= 1;
                    if !gt(v, j, i, cmp) {
                        j += 1;
                        break;
                    }
                }
                v[j..=i].rotate_right(1);
            }
            for i in sentry..n {
                let mut j = i - 1;
                if !gt(v, j, i, cmp) {
                    continue;
                }
                loop {
                    j -= 2;
                    if !gt(v, j, i, cmp) {
                        j += 1;
                        if !gt(v, j, i, cmp) {
                            j += 1;
                        }
                        break;
                    }
                    if j == 0 {
                        break;
                    }
                    if j == 1 {
                        j -= 1;
                        if gt(v, i, j, cmp) {
                            j += 1;
                        }
                        break;
                    }
                }
                v[j..=i].rotate_right(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::zend_sort;

    #[test]
    fn sorts_like_a_total_order_where_there_is_one() {
        for n in [0usize, 1, 2, 5, 6, 7, 16, 17, 100, 1500, 5000] {
            let mut v: Vec<u64> = (0..n as u64).map(|i| i.wrapping_mul(2654435761) % 1000).collect();
            let mut want = v.clone();
            want.sort();
            zend_sort(&mut v, &mut |a, b| a.cmp(b));
            assert_eq!(v, want, "n = {n}");
        }
    }

    #[test]
    fn never_panics_on_an_inconsistent_comparison() {
        let mut v: Vec<u32> = (0..3000).collect();
        let mut flip = 0u32;
        zend_sort(&mut v, &mut |a, b| {
            flip = flip.wrapping_mul(1103515245).wrapping_add(12345);
            if (a ^ b ^ flip) & 4 == 0 { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }
        });
        assert_eq!(v.len(), 3000);
    }
}
