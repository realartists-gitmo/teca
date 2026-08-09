/// Prime-power finite-field orders `q=p^k <= max`, sorted by q then p/k.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PrimePower {
    pub characteristic: u32,
    pub degree: u32,
    pub order: u32,
}

pub fn prime_powers_up_to(max: u32) -> Vec<PrimePower> {
    let mut out = Vec::new();
    for p in 2..=max {
        if !is_prime(p) {
            continue;
        }
        let mut q = p as u64;
        let mut degree = 1u32;
        while q <= max as u64 {
            out.push(PrimePower {
                characteristic: p,
                degree,
                order: q as u32,
            });
            let Some(next) = q.checked_mul(p as u64) else {
                break;
            };
            q = next;
            degree += 1;
        }
    }
    out.sort_by_key(|x| (x.order, x.characteristic, x.degree));
    out
}

pub fn is_prime(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    if n.is_multiple_of(2) {
        return n == 2;
    }
    let mut d = 3u32;
    while (d as u64) * (d as u64) <= n as u64 {
        if n.is_multiple_of(d) {
            return false;
        }
        d += 2;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontier_capacity_contains_107_squared() {
        let fields = prime_powers_up_to(11_456);
        assert!(
            fields
                .iter()
                .any(|x| x.characteristic == 107 && x.degree == 2 && x.order == 11_449)
        );
    }

    #[test]
    fn lower_endpoint_is_two() {
        let fields = prime_powers_up_to(2);
        assert_eq!(
            fields,
            vec![PrimePower {
                characteristic: 2,
                degree: 1,
                order: 2
            }]
        );
    }
}
