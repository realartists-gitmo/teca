/// Binomial coefficient `n choose k` reduced modulo prime `p`, using Lucas'
/// theorem so Hasse orders >= characteristic remain correct.
pub fn binomial_mod_prime(mut n: usize, mut k: usize, p: u32) -> u32 {
    if k > n {
        return 0;
    }
    let mut out = 1u64;
    let p_usize = p as usize;
    while n != 0 || k != 0 {
        let ni = n % p_usize;
        let ki = k % p_usize;
        if ki > ni {
            return 0;
        }
        out = out * small_binomial_mod_prime(ni, ki, p) as u64 % p as u64;
        n /= p_usize;
        k /= p_usize;
    }
    out as u32
}

fn small_binomial_mod_prime(n: usize, k: usize, p: u32) -> u32 {
    let k = k.min(n - k);
    let mut numerator = 1u64;
    let mut denominator = 1u64;
    for i in 0..k {
        numerator = numerator * (n - i) as u64 % p as u64;
        denominator = denominator * (i + 1) as u64 % p as u64;
    }
    let inv = mod_pow(denominator, p as u64 - 2, p as u64);
    (numerator * inv % p as u64) as u32
}

fn mod_pow(mut base: u64, mut exp: u64, modulus: u64) -> u64 {
    let mut out = 1u64;
    while exp != 0 {
        if exp & 1 != 0 {
            out = out * base % modulus;
        }
        base = base * base % modulus;
        exp >>= 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucas_handles_characteristic_crossing() {
        // C(107,1) == 0 mod 107; C(107,107) == 1.
        assert_eq!(binomial_mod_prime(107, 1, 107), 0);
        assert_eq!(binomial_mod_prime(107, 107, 107), 1);
    }
}
