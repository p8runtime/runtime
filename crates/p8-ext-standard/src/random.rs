//! Randomness: mt_rand and rand (PHP's MT19937, so a seed gives PHP's
//! sequence), mt_srand/srand, shuffle, str_shuffle, array_rand,
//! random_int, random_bytes, lcg_value, uniqid.
//!
//! Reference: $PHP_SRC_PATH/ext/random/mt19937.c, random.c,
//! $PHP_SRC_PATH/ext/standard/array.c (php_array_data_shuffle, array_rand)

use crate::{arg, str_arg};
use p8_runtime::array::{Key, PhpArray};
use p8_runtime::ext::{Builtin, Ctx, Throw};
use p8_runtime::string::PhpStr;
use p8_runtime::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

pub static BUILTINS: &[Builtin] = &[
    Builtin { name: "mt_rand", run: mt_rand, by_ref: &[] },
    Builtin { name: "rand", run: mt_rand, by_ref: &[] },
    Builtin { name: "mt_srand", run: mt_srand, by_ref: &[] },
    Builtin { name: "srand", run: mt_srand, by_ref: &[] },
    Builtin { name: "mt_getrandmax", run: getrandmax, by_ref: &[] },
    Builtin { name: "getrandmax", run: getrandmax, by_ref: &[] },
    Builtin { name: "shuffle", run: shuffle, by_ref: &[0] },
    Builtin { name: "str_shuffle", run: str_shuffle, by_ref: &[] },
    Builtin { name: "array_rand", run: array_rand, by_ref: &[] },
    Builtin { name: "random_int", run: random_int, by_ref: &[] },
    Builtin { name: "random_bytes", run: random_bytes, by_ref: &[] },
    Builtin { name: "lcg_value", run: lcg_value, by_ref: &[] },
    Builtin { name: "uniqid", run: uniqid, by_ref: &[] },
];

const N: usize = 624;
const M: usize = 397;

/// PHP's Mersenne Twister state.
struct Mt {
    state: [u32; N],
    left: usize,
    next: usize,
}

impl Mt {
    fn seeded(seed: u32) -> Mt {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            state[i] = 1_812_433_253u32.wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30)).wrapping_add(i as u32);
        }
        let mut mt = Mt { state, left: 0, next: 0 };
        mt.reload();
        mt
    }

    fn reload(&mut self) {
        let twist = |m: u32, u: u32, v: u32| m ^ ((((u & 0x8000_0000) | (v & 0x7fff_ffff)) >> 1) ^ (0u32.wrapping_sub(v & 1) & 0x9908_b0df));
        let s = &mut self.state;
        for i in 0..N - M {
            s[i] = twist(s[i + M], s[i], s[i + 1]);
        }
        for i in N - M..N - 1 {
            s[i] = twist(s[i + M - N], s[i], s[i + 1]);
        }
        s[N - 1] = twist(s[M - 1], s[N - 1], s[0]);
        self.left = N;
        self.next = 0;
    }

    fn next_u32(&mut self) -> u32 {
        if self.left == 0 {
            self.reload();
        }
        self.left -= 1;
        let mut s1 = self.state[self.next];
        self.next += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }

    /// php_mt_rand_range: uniform in min..=max.
    fn range(&mut self, min: i64, max: i64) -> i64 {
        let umax = max.wrapping_sub(min) as u64;
        if umax > u64::from(u32::MAX) {
            // The first draw is the low half (php_random_range64).
            let draw = |m: &mut Mt| {
                let low = u64::from(m.next_u32());
                low | (u64::from(m.next_u32()) << 32)
            };
            let mut result = draw(self);
            if umax != u64::MAX {
                let umax = umax + 1;
                if umax & (umax - 1) != 0 {
                    let limit = u64::MAX - (u64::MAX % umax) - 1;
                    while result > limit {
                        result = draw(self);
                    }
                }
                result %= umax;
            }
            return min.wrapping_add(result as i64);
        }
        let umax = umax as u32;
        let mut result = self.next_u32();
        if umax != u32::MAX {
            let umax = umax + 1;
            if umax & (umax - 1) != 0 {
                let limit = u32::MAX - (u32::MAX % umax) - 1;
                while result > limit {
                    result = self.next_u32();
                }
            }
            result %= umax;
        }
        min.wrapping_add(i64::from(result))
    }
}

fn os_random<const K: usize>() -> [u8; K] {
    let mut b = [0u8; K];
    let _ = getrandom::getrandom(&mut b);
    b
}

thread_local! {
    static MT: RefCell<Option<Mt>> = const { RefCell::new(None) };
}

/// The generator (seeded from the OS the first time, as PHP does).
fn with_mt<R>(f: impl FnOnce(&mut Mt) -> R) -> R {
    MT.with(|m| {
        let mut m = m.borrow_mut();
        let mt = m.get_or_insert_with(|| Mt::seeded(u32::from_le_bytes(os_random::<4>())));
        f(mt)
    })
}

/// mt_rand(int $min, int $max) / mt_rand(): 31 random bits.
fn mt_rand(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    if args.is_empty() {
        return Ok(Value::Int(i64::from(with_mt(|m| m.next_u32() >> 1))));
    }
    let (min, max) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    if max < min {
        return Err(ctx.throw("ValueError", "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)".into()));
    }
    Ok(Value::Int(with_mt(|m| m.range(min, max))))
}

/// mt_srand(int $seed = 0, int $mode = MT_RAND_MT19937)
fn mt_srand(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let seed = match args.first().map(Value::deref) {
        Some(Value::Null) | None => u32::from_le_bytes(os_random::<4>()),
        Some(v) => v.to_int() as u32,
    };
    MT.with(|m| *m.borrow_mut() = Some(Mt::seeded(seed)));
    Ok(Value::Null)
}

fn getrandmax(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Int(2_147_483_647))
}

/// Fisher-Yates from the end, as php_array_data_shuffle does.
fn shuffle_in_place<T>(items: &mut [T]) {
    if items.len() < 2 {
        return;
    }
    with_mt(|m| {
        for j in (1..items.len()).rev() {
            let k = m.range(0, j as i64) as usize;
            items.swap(j, k);
        }
    });
}

fn shuffle(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Some(Value::Ref(cell)) = args.first() else { return Ok(Value::Bool(true)) };
    let mut value = cell.borrow_mut();
    if let Value::Array(a) = &*value {
        let mut items: Vec<Value> = a.values().cloned().collect();
        shuffle_in_place(&mut items);
        *value = Value::Array(Rc::new(PhpArray::from_list(items)));
    }
    Ok(Value::Bool(true))
}

fn str_shuffle(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let mut bytes = str_arg(args, 0).as_bytes().to_vec();
    shuffle_in_place(&mut bytes);
    Ok(Value::Str(PhpStr::new(bytes)))
}

/// array_rand(array $array, int $num = 1): a key, or `num` keys in order.
fn array_rand(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let Value::Array(a) = arg(args, 0) else { return Ok(Value::Null) };
    let keys: Vec<Key> = a.iter().map(|(k, _)| k).collect();
    let n = keys.len();
    if n == 0 {
        return Err(ctx.throw("ValueError", "array_rand(): Argument #1 ($array) cannot be empty".into()));
    }
    let num = if args.len() > 1 { arg(args, 1).to_int() } else { 1 };
    if num <= 0 || num as usize > n {
        return Err(ctx.throw(
            "ValueError",
            "array_rand(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)".into(),
        ));
    }
    if num == 1 {
        let i = with_mt(|m| m.range(0, n as i64 - 1)) as usize;
        return Ok(keys[i].to_value());
    }
    // Pick the fewer: the keys to take, or those to leave.
    let num = num as usize;
    let negative = num > n / 2;
    let mut want = if negative { n - num } else { num };
    let mut picked = vec![false; n];
    with_mt(|m| {
        while want > 0 {
            let i = m.range(0, n as i64 - 1) as usize;
            if !picked[i] {
                picked[i] = true;
                want -= 1;
            }
        }
    });
    let out: Vec<Value> = keys.iter().zip(picked).filter(|(_, p)| *p != negative).map(|(k, _)| k.to_value()).collect();
    Ok(Value::Array(Rc::new(PhpArray::from_list(out))))
}

/// random_int(int $min, int $max): from the OS's randomness.
fn random_int(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let (min, max) = (arg(args, 0).to_int(), arg(args, 1).to_int());
    if min > max {
        return Err(ctx.throw("ValueError", "random_int(): Argument #1 ($min) must be less than or equal to argument #2 ($max)".into()));
    }
    let span = max.wrapping_sub(min) as u64;
    if span == u64::MAX {
        return Ok(Value::Int(i64::from_le_bytes(os_random::<8>())));
    }
    let range = span + 1;
    let limit = u64::MAX - (u64::MAX % range);
    loop {
        let r = u64::from_le_bytes(os_random::<8>());
        if r < limit {
            return Ok(Value::Int(min.wrapping_add((r % range) as i64)));
        }
    }
}

fn random_bytes(ctx: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let n = arg(args, 0).to_int();
    if n < 1 {
        return Err(ctx.throw("ValueError", "random_bytes(): Argument #1 ($length) must be greater than 0".into()));
    }
    let mut b = vec![0u8; n as usize];
    let _ = getrandom::getrandom(&mut b);
    Ok(Value::Str(PhpStr::new(b)))
}

fn lcg_value(_: &mut dyn Ctx, _: &mut [Value]) -> Result<Value, Throw> {
    Ok(Value::Float(f64::from(u32::from_le_bytes(os_random::<4>())) / 4_294_967_296.0))
}

/// uniqid(string $prefix = "", bool $more_entropy = false)
fn uniqid(_: &mut dyn Ctx, args: &mut [Value]) -> Result<Value, Throw> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let mut out = String::from_utf8_lossy(str_arg(args, 0).as_bytes()).into_owned();
    out.push_str(&format!("{:08x}{:05x}", now.as_secs(), now.subsec_micros()));
    if args.len() > 1 && arg(args, 1).to_bool() {
        let r = f64::from(u32::from_le_bytes(os_random::<4>())) / 429_496_729.6;
        out.push_str(&format!("{r:.8}"));
    }
    // Two calls in one microsecond would be equal: PHP sleeps a little.
    std::thread::sleep(std::time::Duration::from_micros(1));
    Ok(Value::Str(PhpStr::from(out)))
}
