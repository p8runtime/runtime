//! ext/random's global functions and the request's Mt19937 engine, ported
//! from php-src (engine_mt19937.c, random.c) so a seeded sequence matches
//! PHP's exactly: mt_rand(), rand(), shuffle(), str_shuffle() and
//! array_rand() all draw from it.

use crate::core::value::{Handle, Val};
use crate::vm::engine::{ErrorLevel, VM};
use rand::RngCore;
use std::rc::Rc;

const N: usize = 624;
const M: usize = 397;

/// MT_RAND_MT19937 (the correct algorithm) or MT_RAND_PHP (PHP 5's
/// variant with a modulo-biased range and a wrong twist).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MtMode {
    Mt19937,
    Php,
}

/// The largest value mt_rand() and rand() return without arguments.
pub const MT_RAND_MAX: i64 = 0x7FFF_FFFF;

#[derive(Clone, Debug)]
pub struct Mt19937 {
    state: [u32; N],
    count: usize,
    pub mode: MtMode,
}

impl Mt19937 {
    /// php_random_mt19937_seed32
    pub fn seeded(seed: u32, mode: MtMode) -> Self {
        let mut mt = Self {
            state: [0; N],
            count: N,
            mode,
        };
        mt.state[0] = seed;
        for i in 1..N {
            let prev = mt.state[i - 1];
            mt.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        mt.reload();
        mt
    }

    fn reload(&mut self) {
        let mode = self.mode;
        let twist = |m: u32, u: u32, v: u32| -> u32 {
            let mix = (u & 0x8000_0000) | (v & 0x7FFF_FFFF);
            let low = match mode {
                MtMode::Mt19937 => v & 1,
                MtMode::Php => u & 1,
            };
            m ^ (mix >> 1) ^ (0u32.wrapping_sub(low) & 0x9908_B0DF)
        };
        let s = &mut self.state;
        for i in 0..N - M {
            s[i] = twist(s[i + M], s[i], s[i + 1]);
        }
        for i in N - M..N - 1 {
            s[i] = twist(s[i + M - N], s[i], s[i + 1]);
        }
        s[N - 1] = twist(s[M - 1], s[N - 1], s[0]);
        self.count = 0;
    }

    /// One 32-bit output (the engine's generate()).
    pub fn next_u32(&mut self) -> u32 {
        if self.count >= N {
            self.reload();
        }
        let mut s1 = self.state[self.count];
        self.count += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9D2C_5680;
        s1 ^= (s1 << 15) & 0xEFC6_0000;
        s1 ^ (s1 >> 18)
    }

    /// A uniform value in [min, max] (php_random_range).
    pub fn range(&mut self, min: i64, max: i64) -> i64 {
        range_with(min, max, || self.next_u32() as u64, 4)
    }
}

/// php_random_range over any engine: `next` yields `size` bytes at a
/// time; 32-bit draws for spans that fit, 64-bit otherwise, rejecting the
/// values that would bias the modulo.
fn range_with(min: i64, max: i64, mut next: impl FnMut() -> u64, size: u32) -> i64 {
    let umax = (max as u64).wrapping_sub(min as u64);
    let mut draw = |bytes: u32| -> u64 {
        let mut result = 0u64;
        let mut total = 0;
        while total < bytes {
            result |= next() << (total * 8);
            total += size;
        }
        result
    };
    if umax > u32::MAX as u64 {
        let mut result = draw(8);
        if umax == u64::MAX {
            return (result as i64).wrapping_add(min);
        }
        let umax = umax + 1;
        if umax & (umax - 1) == 0 {
            return ((result & (umax - 1)) as i64).wrapping_add(min);
        }
        let limit = u64::MAX - (u64::MAX % umax) - 1;
        while result > limit {
            result = draw(8);
        }
        return ((result % umax) as i64).wrapping_add(min);
    }
    let umax = umax as u32;
    let mut result = draw(4) as u32;
    if umax == u32::MAX {
        return (result as i64).wrapping_add(min);
    }
    let umax = umax + 1;
    if umax & (umax - 1) == 0 {
        return ((result & (umax - 1)) as i64).wrapping_add(min);
    }
    let limit = u32::MAX - (u32::MAX % umax) - 1;
    while result > limit {
        result = draw(4) as u32;
    }
    ((result % umax) as i64).wrapping_add(min)
}

/// The request's default engine, seeded from the CSPRNG on first use
/// unless mt_srand() seeded it.
#[derive(Default)]
struct RandomState {
    mt: Option<Mt19937>,
}

/// The request's Mt19937 (php_random_default_status).
pub fn default_engine(vm: &mut VM) -> &mut Mt19937 {
    let state = vm.context.get_or_init_extension_data(RandomState::default);
    state
        .mt
        .get_or_insert_with(|| Mt19937::seeded(rand::rngs::OsRng.next_u32(), MtMode::Mt19937))
}

fn int_arg(vm: &VM, args: &[Handle], i: usize) -> i64 {
    args.get(i).map_or(0, |&h| vm.arena.get(h).value.to_int())
}

/// mt_srand(?int $seed = null, int $mode = MT_RAND_MT19937): void; srand() is an alias.
pub fn php_mt_srand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let seed = match args.first().map(|&h| vm.arena.get(h).value.clone()) {
        None | Some(Val::Null) => None,
        Some(v) => Some(v.to_int()),
    };
    let mode = match int_arg(vm, args, 1) {
        1 => {
            vm.report_error(ErrorLevel::Deprecated, "The MT_RAND_PHP variant of Mt19937 is deprecated");
            MtMode::Php
        }
        _ => MtMode::Mt19937,
    };
    let seed = seed.map_or_else(|| rand::rngs::OsRng.next_u32(), |s| s as u32);
    let state = vm.context.get_or_init_extension_data(RandomState::default);
    state.mt = Some(Mt19937::seeded(seed, mode));
    Ok(vm.arena.alloc(Val::Null))
}

/// php_mt_rand_common: the MT_RAND_PHP mode scales instead of using a
/// uniform range.
fn mt_rand_common(vm: &mut VM, min: i64, max: i64) -> i64 {
    let mt = default_engine(vm);
    if mt.mode == MtMode::Mt19937 {
        return mt.range(min, max);
    }
    let r = (mt.next_u32() >> 1) as f64;
    let offset = ((max as f64 - min as f64 + 1.0) * (r / (MT_RAND_MAX as f64 + 1.0))) as u64;
    (offset as i64).wrapping_add(min)
}

/// mt_rand()/rand() with no arguments, or the arity error otherwise.
fn two_args(vm: &mut VM, args: &[Handle], name: &str) -> Result<Option<(i64, i64)>, String> {
    match args.len() {
        0 => Ok(None),
        2 => Ok(Some((int_arg(vm, args, 0), int_arg(vm, args, 1)))),
        n => Err(vm.throw_from_native(
            b"ArgumentCountError",
            &format!("{}() expects exactly 2 arguments, {} given", name, n),
        )),
    }
}

/// mt_rand(int $min = UNKNOWN, int $max = UNKNOWN): int
pub fn php_mt_rand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let value = match two_args(vm, args, "mt_rand")? {
        None => (default_engine(vm).next_u32() >> 1) as i64,
        Some((min, max)) if max < min => {
            return Err(vm.throw_from_native(
                b"ValueError",
                "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)",
            ));
        }
        Some((min, max)) => mt_rand_common(vm, min, max),
    };
    Ok(vm.arena.alloc(Val::Int(value)))
}

/// rand(int $min = UNKNOWN, int $max = UNKNOWN): int — min > max is allowed.
pub fn php_rand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let value = match two_args(vm, args, "rand")? {
        None => (default_engine(vm).next_u32() >> 1) as i64,
        Some((min, max)) if max < min => mt_rand_common(vm, max, min),
        Some((min, max)) => mt_rand_common(vm, min, max),
    };
    Ok(vm.arena.alloc(Val::Int(value)))
}

/// mt_getrandmax(): int; getrandmax() is an alias.
pub fn php_mt_getrandmax(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    Ok(vm.arena.alloc(Val::Int(MT_RAND_MAX)))
}

/// random_int(int $min, int $max): int, from the OS CSPRNG.
pub fn php_random_int(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let (min, max) = (int_arg(vm, args, 0), int_arg(vm, args, 1));
    if min > max {
        return Err(vm.throw_from_native(
            b"ValueError",
            "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)",
        ));
    }
    let value = range_with(min, max, || rand::rngs::OsRng.next_u64(), 8);
    Ok(vm.arena.alloc(Val::Int(value)))
}

/// random_bytes(int $length): string, from the OS CSPRNG.
pub fn php_random_bytes(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let length = int_arg(vm, args, 0);
    if length < 1 {
        return Err(vm.throw_from_native(
            b"ValueError",
            "random_bytes(): Argument #1 ($length) must be greater than 0",
        ));
    }
    let mut bytes = vec![0u8; length as usize];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    Ok(vm.arena.alloc(Val::String(Rc::new(bytes))))
}

/// lcg_value(): float in (0, 1), from PHP's combined linear congruential
/// generator (lcg.c), seeded per request from the CSPRNG.
pub fn php_lcg_value(vm: &mut VM, _args: &[Handle]) -> Result<Handle, String> {
    #[derive(Default)]
    struct Lcg(Option<(i32, i32)>);
    let lcg = vm.context.get_or_init_extension_data(Lcg::default);
    let (mut s1, mut s2) = *lcg.0.get_or_insert_with(|| {
        let r = rand::rngs::OsRng.next_u64();
        (((r as u32) % 2_147_483_562) as i32 + 1, (((r >> 32) as u32) % 2_147_483_398) as i32 + 1)
    });
    // MODMULT(a, b, c, m, s): q = s / a; s = b * (s - a * q) - c * q; if s < 0 then s += m
    let modmult = |a: i32, b: i32, c: i32, m: i32, s: i32| -> i32 {
        let q = s / a;
        let s = b * (s - a * q) - c * q;
        if s < 0 { s + m } else { s }
    };
    s1 = modmult(53668, 40014, 12211, 2_147_483_563, s1);
    s2 = modmult(52774, 40692, 3791, 2_147_483_399, s2);
    lcg.0 = Some((s1, s2));
    let mut z = s1 - s2;
    if z < 1 {
        z += 2_147_483_562;
    }
    Ok(vm.arena.alloc(Val::Float(z as f64 * 4.656613e-10)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt19937_matches_the_reference_sequence() {
        // mt19937ar.c's init_genrand(5489) output, which PHP's engine shares
        let mut mt = Mt19937::seeded(5489, MtMode::Mt19937);
        let first: Vec<u32> = (0..3).map(|_| mt.next_u32()).collect();
        assert_eq!(first, [3499211612, 581869302, 3890346734]);
    }
}
