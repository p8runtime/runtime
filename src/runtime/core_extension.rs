use crate::builtins::{
    array, bcmath, class, exception, exec, filesystem, function, http, math, output_control, pcre,
    reflection, spl, string, url, variable, wasm,
};
use crate::core::value::{Handle, Val, Visibility};
use crate::runtime::context::RequestContext;
use crate::runtime::extension::{Extension, ExtensionInfo, ExtensionResult};
use crate::runtime::registry::{ExtensionRegistry, NativeClassDef, NativeMethodEntry};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Core extension runtime state
#[derive(Debug, Default)]
pub struct CoreExtensionData {
    pub strtok_string: Option<Vec<u8>>,
    pub strtok_pos: usize,
    pub array_pointers: HashMap<Handle, usize>,
    pub rng: Mt19937,
}

#[derive(Clone, Copy, Debug)]
pub struct Mt19937 {
    mt: [u32; 624],
    index: usize,
    seeded: bool,
}

impl Default for Mt19937 {
    fn default() -> Self {
        Self {
            mt: [0; 624],
            index: 624,
            seeded: false,
        }
    }
}

impl Mt19937 {
    fn seed(&mut self, seed: u32) {
        self.mt[0] = seed;
        for i in 1..624 {
            let prev = self.mt[i - 1];
            self.mt[i] = 1812433253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = 624;
        self.seeded = true;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            self.twist();
        }

        let mut y = self.mt[self.index];
        self.index += 1;

        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c5680;
        y ^= (y << 15) & 0xefc60000;
        y ^= y >> 18;

        y
    }

    fn twist(&mut self) {
        const N: usize = 624;
        const M: usize = 397;
        const MATRIX_A: u32 = 0x9908b0df;
        const UPPER_MASK: u32 = 0x80000000;
        const LOWER_MASK: u32 = 0x7fffffff;

        for i in 0..N {
            let y = (self.mt[i] & UPPER_MASK) | (self.mt[(i + 1) % N] & LOWER_MASK);
            let mut val = self.mt[(i + M) % N] ^ (y >> 1);
            if y & 1 != 0 {
                val ^= MATRIX_A;
            }
            self.mt[i] = val;
        }
        self.index = 0;
    }
}

impl CoreExtensionData {
    pub fn rng_seed(&mut self, seed: u32) {
        self.rng.seed(seed);
    }

    pub fn rng_next_u31(&mut self) -> u32 {
        self.ensure_rng_seeded();
        self.rng.next_u32() >> 1
    }

    pub fn rng_next_u32(&mut self) -> u32 {
        self.ensure_rng_seeded();
        self.rng.next_u32()
    }

    fn ensure_rng_seeded(&mut self) {
        if self.rng.seeded {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        let seed = 0u64;
        #[cfg(not(target_arch = "wasm32"))]
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        self.rng.seed(seed as u32);
    }
}

/// Core extension providing built-in PHP functions
pub struct CoreExtension;

impl Extension for CoreExtension {
    fn info(&self) -> ExtensionInfo {
        ExtensionInfo {
            name: "Core",
            version: "8.3.0",
            dependencies: &[],
        }
    }

    fn module_init(&self, registry: &mut ExtensionRegistry) -> ExtensionResult {
        // String functions
        registry.register_function(b"strlen", string::php_strlen);
        registry.register_function(b"str_repeat", string::php_str_repeat);
        registry.register_function(b"substr", string::php_substr);
        registry.register_function(b"substr_replace", string::php_substr_replace);
        registry.register_function(b"strpos", string::php_strpos);
        // registry.register_function(b"stripos", string::php_stripos);
        // registry.register_function(b"strrpos", string::php_strrpos);
        // registry.register_function(b"strripos", string::php_strripos);
        // registry.register_function(b"strrchr", string::php_strrchr);
        // registry.register_function(b"strpbrk", string::php_strpbrk);
        // registry.register_function(b"strspn", string::php_strspn);
        // registry.register_function(b"strcspn", string::php_strcspn);
        // registry.register_function(b"strtr", string::php_strtr);
        registry.register_function(b"__deka_chr", string::php_deka_chr);
        registry.register_function(b"__deka_ord", string::php_deka_ord);
        registry.register_function(b"__deka_crc32", string::php_deka_crc32);
        registry.register_function(b"__deka_md5", string::php_deka_md5);
        registry.register_function(b"__deka_md5_file", string::php_deka_md5_file);
        registry.register_function(b"__deka_sha1", string::php_deka_sha1);
        registry.register_function(b"__deka_sha1_file", string::php_deka_sha1_file);
        registry.register_function(b"trim", string::php_trim);
        registry.register_function(b"ltrim", string::php_ltrim);
        registry.register_function(b"rtrim", string::php_rtrim);
        registry.register_function(b"chop", string::php_rtrim);
        registry.register_function(b"chr", string::php_chr);
        registry.register_function(b"ord", string::php_ord);
        // registry.register_function(b"bin2hex", string::php_bin2hex);
        // registry.register_function(b"hex2bin", string::php_hex2bin);
        // registry.register_function(b"crc32", string::php_crc32);
        // registry.register_function(b"md5", string::php_md5);
        // registry.register_function(b"md5_file", string::php_md5_file);
        // registry.register_function(b"sha1", string::php_sha1);
        // registry.register_function(b"sha1_file", string::php_sha1_file);
        registry.register_function(b"__deka_crypt", string::php_deka_crypt);
        // registry.register_function(b"crypt", string::php_crypt);
        registry.register_function(
            b"quoted_printable_decode",
            string::php_quoted_printable_decode,
        );
        registry.register_function(
            b"quoted_printable_encode",
            string::php_quoted_printable_encode,
        );
        // registry.register_function(
        //     b"__deka_convert_cyr_string",
        //     string::php_deka_convert_cyr_string,
        // );
        // registry.register_function(
        //     b"__deka_convert_uuencode",
        //     string::php_deka_convert_uuencode,
        // );
        // registry.register_function(
        //     b"__deka_convert_uudecode",
        //     string::php_deka_convert_uudecode,
        // );
        // registry.register_function(b"convert_cyr_string", string::php_convert_cyr_string);
        // registry.register_function(b"convert_uuencode", string::php_convert_uuencode);
        // registry.register_function(b"convert_uudecode", string::php_convert_uudecode);
        // registry.register_function(b"addslashes", string::php_addslashes);
        // registry.register_function(b"stripslashes", string::php_stripslashes);
        // registry.register_function(b"addcslashes", string::php_addcslashes);
        // registry.register_function(b"stripcslashes", string::php_stripcslashes);
        // registry.register_function(b"str_pad", string::php_str_pad);
        // registry.register_function(b"str_rot13", string::php_str_rot13);
        // registry.register_function(b"str_shuffle", string::php_str_shuffle);
        // registry.register_function(b"str_split", string::php_str_split);
        // registry.register_function(b"chunk_split", string::php_chunk_split);
        // registry.register_function(b"str_getcsv", string::php_str_getcsv);
        // registry.register_function(b"strrev", string::php_strrev);
        // registry.register_function(b"__deka_metaphone", string::php_deka_metaphone);
        // registry.register_function(b"__deka_setlocale", string::php_deka_setlocale);
        // registry.register_function(b"__deka_localeconv", string::php_deka_localeconv);
        // registry.register_function(b"__deka_nl_langinfo", string::php_deka_nl_langinfo);
        // registry.register_function(b"__deka_strcoll", string::php_deka_strcoll);
        // registry.register_function(b"__deka_number_format", string::php_deka_number_format);
        // registry.register_function(b"__deka_money_format", string::php_deka_money_format);
        // registry.register_function(b"__deka_strcmp", string::php_deka_strcmp);
        // registry.register_function(b"__deka_strcasecmp", string::php_deka_strcasecmp);
        // registry.register_function(b"__deka_strncmp", string::php_deka_strncmp);
        // registry.register_function(b"__deka_strncasecmp", string::php_deka_strncasecmp);
        // registry.register_function(b"__deka_strnatcmp", string::php_deka_strnatcmp);
        // registry.register_function(b"__deka_strnatcasecmp", string::php_deka_strnatcasecmp);
        // registry.register_function(b"__deka_levenshtein", string::php_deka_levenshtein);
        // registry.register_function_with_by_ref(
        //     b"__deka_similar_text",
        //     string::php_deka_similar_text,
        //     vec![2],
        // );
        // registry.register_function(b"__deka_soundex", string::php_deka_soundex);
        // registry.register_function(b"__deka_substr_compare", string::php_deka_substr_compare);
        // registry.register_function(b"__deka_strstr", string::php_deka_strstr);
        // registry.register_function(b"__deka_stristr", string::php_deka_stristr);
        // registry.register_function(b"metaphone", string::php_metaphone);
        // registry.register_function(b"setlocale", string::php_setlocale);
        // registry.register_function(b"localeconv", string::php_localeconv);
        // registry.register_function(b"nl_langinfo", string::php_nl_langinfo);
        // registry.register_function(b"strcoll", string::php_strcoll);
        // registry.register_function(b"number_format", string::php_number_format);
        // registry.register_function(b"money_format", string::php_money_format);
        // registry.register_function(b"strcmp", string::php_strcmp);
        // registry.register_function(b"strcasecmp", string::php_strcasecmp);
        // registry.register_function(b"strncmp", string::php_strncmp);
        // registry.register_function(b"strncasecmp", string::php_strncasecmp);
        // registry.register_function(b"strnatcmp", string::php_strnatcmp);
        // registry.register_function(b"strnatcasecmp", string::php_strnatcasecmp);
        // registry.register_function(b"levenshtein", string::php_levenshtein);
        // registry.register_function_with_by_ref(b"similar_text", string::php_similar_text, vec![2]);
        // registry.register_function(b"soundex", string::php_soundex);
        // registry.register_function(b"substr_compare", string::php_substr_compare);
        // registry.register_function(b"strstr", string::php_strstr);
        // registry.register_function(b"stristr", string::php_stristr);
        // registry.register_function(b"substr_count", string::php_substr_count);
        // registry.register_function(b"ucfirst", string::php_ucfirst);
        // registry.register_function(b"lcfirst", string::php_lcfirst);
        // registry.register_function(b"ucwords", string::php_ucwords);
        // registry.register_function(b"__deka_hebrev", string::php_deka_hebrev);
        // registry.register_function(b"__deka_wordwrap", string::php_deka_wordwrap);
        // registry.register_function(b"__deka_quotemeta", string::php_deka_quotemeta);
        // registry.register_function(b"__deka_nl2br", string::php_deka_nl2br);
        // registry.register_function(b"__deka_strip_tags", string::php_deka_strip_tags);
        // registry.register_function(b"__deka_strtok", string::php_deka_strtok);
        // registry.register_function(b"__deka_count_chars", string::php_deka_count_chars);
        // registry.register_function(b"__deka_str_word_count", string::php_deka_str_word_count);
        // registry.register_function(b"hebrev", string::php_hebrev);
        // registry.register_function(b"wordwrap", string::php_wordwrap);
        // registry.register_function(b"quotemeta", string::php_quotemeta);
        // registry.register_function(b"nl2br", string::php_nl2br);
        // registry.register_function(b"strip_tags", string::php_strip_tags);
        // registry.register_function(b"strtok", string::php_strtok);
        // registry.register_function(b"count_chars", string::php_count_chars);
        // registry.register_function(b"str_word_count", string::php_str_word_count);
        // registry.register_function(b"str_contains", string::php_str_contains);
        // registry.register_function(b"str_starts_with", string::php_str_starts_with);
        // registry.register_function(b"str_ends_with", string::php_str_ends_with);
        // registry.register_function(b"__deka_str_increment", string::php_deka_str_increment);
        // registry.register_function(b"__deka_str_decrement", string::php_deka_str_decrement);
        // registry.register_function(b"__deka_htmlspecialchars", string::php_deka_htmlspecialchars);
        // registry.register_function(
        //     b"__deka_htmlspecialchars_decode",
        //     string::php_deka_htmlspecialchars_decode,
        // );
        // registry.register_function(b"__deka_htmlentities", string::php_deka_htmlentities);
        // registry.register_function(b"__deka_html_entity_decode", string::php_deka_html_entity_decode);
        registry.register_function(
            b"get_html_translation_table",
            string::php_get_html_translation_table,
        );
        // registry.register_function_with_by_ref(
        //     b"__deka_str_replace",
        //     string::php_deka_str_replace,
        //     vec![3],
        // );
        // registry.register_function_with_by_ref(
        //     b"__deka_str_ireplace",
        //     string::php_deka_str_ireplace,
        //     vec![3],
        // );
        // registry.register_function_with_by_ref(
        //     b"__deka_parse_str",
        //     string::php_deka_parse_str,
        //     vec![1],
        // );
        // registry.register_function(b"str_increment", string::php_str_increment);
        // registry.register_function(b"str_decrement", string::php_str_decrement);
        // registry.register_function(b"htmlspecialchars", string::php_htmlspecialchars);
        // registry.register_function(
        //     b"htmlspecialchars_decode",
        //     string::php_htmlspecialchars_decode,
        // );
        // registry.register_function(b"htmlentities", string::php_htmlentities);
        // registry.register_function(b"html_entity_decode", string::php_html_entity_decode);
        registry.register_function_with_by_ref(b"str_replace", string::php_str_replace, vec![3]);
        // registry.register_function_with_by_ref(b"str_ireplace", string::php_str_ireplace, vec![3]);
        registry.register_function_with_by_ref(b"parse_str", string::php_parse_str, vec![1]);
        registry.register_function(b"strtolower", string::php_strtolower);
        registry.register_function(b"strtoupper", string::php_strtoupper);
        // registry.register_function(b"__deka_utf8_encode", string::php_deka_utf8_encode);
        // registry.register_function(b"__deka_utf8_decode", string::php_deka_utf8_decode);
        // registry.register_function(b"__deka_version_compare", string::php_deka_version_compare);
        // registry.register_function(b"utf8_encode", string::php_utf8_encode);
        // registry.register_function(b"utf8_decode", string::php_utf8_decode);
        // registry.register_function(b"version_compare", string::php_version_compare);
        registry.register_function(b"implode", string::php_implode);
        registry.register_function(b"join", string::php_implode);
        registry.register_function(b"explode", string::php_explode);
        // registry.register_function(b"strchr", string::php_strstr);
        // registry.register_function(b"__deka_sprintf", string::php_deka_sprintf);
        // registry.register_function(b"__deka_sscanf", string::php_deka_sscanf);
        // registry.register_function(b"__deka_printf", string::php_deka_printf);
        // registry.register_function(b"__deka_vsprintf", string::php_deka_vsprintf);
        // registry.register_function(b"__deka_vprintf", string::php_deka_vprintf);
        // registry.register_function(b"__deka_fprintf", string::php_deka_fprintf);
        // registry.register_function(b"__deka_vfprintf", string::php_deka_vfprintf);
        // registry.register_function(b"sprintf", string::php_sprintf);
        // registry.register_function(b"sscanf", string::php_sscanf);
        // registry.register_function(b"printf", string::php_printf);
        // registry.register_function(b"vsprintf", string::php_vsprintf);
        // registry.register_function(b"vprintf", string::php_vprintf);
        // registry.register_function(b"fprintf", string::php_fprintf);
        // registry.register_function(b"vfprintf", string::php_vfprintf);

        // Array functions
        // registry.register_function(b"__deka_array", array::php_deka_array);
        // registry.register_function(b"__deka_array_all", array::php_deka_array_all);
        // registry.register_function(b"__deka_array_any", array::php_deka_array_any);
        // registry.register_function(
        //     b"__deka_array_change_key_case",
        //     array::php_deka_array_change_key_case,
        // );
        // registry.register_function(b"__deka_array_chunk", array::php_deka_array_chunk);
        // registry.register_function(b"__deka_array_column", array::php_deka_array_column);
        // registry.register_function(b"__deka_array_combine", array::php_deka_array_combine);
        // registry.register_function(
        //     b"__deka_array_count_values",
        //     array::php_deka_array_count_values,
        // );
        // registry.register_function(b"__deka_array_diff", array::php_deka_array_diff);
        // registry.register_function(
        //     b"__deka_array_diff_assoc",
        //     array::php_deka_array_diff_assoc,
        // );
        // registry.register_function(b"array", array::php_array);
        // registry.register_function(b"array_all", array::php_array_all);
        // registry.register_function(b"array_any", array::php_array_any);
        // registry.register_function(b"array_change_key_case", array::php_array_change_key_case);
        // registry.register_function(b"array_chunk", array::php_array_chunk);
        // registry.register_function(b"array_column", array::php_array_column);
        // registry.register_function(b"array_combine", array::php_array_combine);
        // registry.register_function(b"array_count_values", array::php_array_count_values);
        // registry.register_function(b"array_diff", array::php_array_diff);
        // registry.register_function(b"array_diff_assoc", array::php_array_diff_assoc);
        // registry.register_function(b"__deka_array_diff_key", array::php_deka_array_diff_key);
        // registry.register_function(
        //     b"__deka_array_diff_uassoc",
        //     array::php_deka_array_diff_uassoc,
        // );
        // registry.register_function(b"__deka_array_diff_ukey", array::php_deka_array_diff_ukey);
        // registry.register_function(b"__deka_array_fill", array::php_deka_array_fill);
        // registry.register_function(b"__deka_array_fill_keys", array::php_deka_array_fill_keys);
        // registry.register_function(b"__deka_array_filter", array::php_deka_array_filter);
        // registry.register_function(b"__deka_array_find", array::php_deka_array_find);
        // registry.register_function(b"__deka_array_find_key", array::php_deka_array_find_key);
        // registry.register_function(b"__deka_array_first", array::php_deka_array_first);
        // registry.register_function(b"array_diff_key", array::php_array_diff_key);
        // registry.register_function(b"array_diff_uassoc", array::php_array_diff_uassoc);
        // registry.register_function(b"array_diff_ukey", array::php_array_diff_ukey);
        // registry.register_function(b"array_fill", array::php_array_fill);
        // registry.register_function(b"array_fill_keys", array::php_array_fill_keys);
        // registry.register_function(b"array_filter", array::php_array_filter);
        // registry.register_function(b"array_find", array::php_array_find);
        // registry.register_function(b"array_find_key", array::php_array_find_key);
        // registry.register_function(b"array_first", array::php_array_first);
        // registry.register_function(b"__deka_array_flip", array::php_deka_array_flip);
        // registry.register_function(b"__deka_array_intersect", array::php_deka_array_intersect);
        // registry.register_function(
        //     b"__deka_array_intersect_assoc",
        //     array::php_deka_array_intersect_assoc,
        // );
        // registry.register_function(
        //     b"__deka_array_intersect_key",
        //     array::php_deka_array_intersect_key,
        // );
        // registry.register_function(
        //     b"__deka_array_intersect_uassoc",
        //     array::php_deka_array_intersect_uassoc,
        // );
        // registry.register_function(
        //     b"__deka_array_intersect_ukey",
        //     array::php_deka_array_intersect_ukey,
        // );
        // registry.register_function(b"__deka_array_is_list", array::php_deka_array_is_list);
        // registry.register_function(b"__deka_array_key_first", array::php_deka_array_key_first);
        // registry.register_function(b"__deka_array_key_last", array::php_deka_array_key_last);
        // registry.register_function(b"array_flip", array::php_array_flip);
        // registry.register_function(b"array_intersect", array::php_array_intersect);
        // registry.register_function(b"array_intersect_assoc", array::php_array_intersect_assoc);
        // registry.register_function(b"array_intersect_key", array::php_array_intersect_key);
        // registry.register_function(b"array_intersect_uassoc", array::php_array_intersect_uassoc);
        // registry.register_function(b"array_intersect_ukey", array::php_array_intersect_ukey);
        // registry.register_function(b"array_is_list", array::php_array_is_list);
        // registry.register_function(b"array_key_first", array::php_array_key_first);
        // registry.register_function(b"array_key_last", array::php_array_key_last);
        // registry.register_function(b"__deka_array_last", array::php_deka_array_last);
        // registry.register_function(b"__deka_array_map", array::php_deka_array_map);
        // registry.register_function(b"__deka_array_merge", array::php_deka_array_merge);
        // registry.register_function(
        //     b"__deka_array_merge_recursive",
        //     array::php_deka_array_merge_recursive,
        // );
        // registry.register_function(b"__deka_array_multisort", array::php_deka_array_multisort);
        // registry.register_function(b"__deka_array_pad", array::php_deka_array_pad);
        // registry.register_function(b"array_last", array::php_array_last);
        // registry.register_function(b"array_map", array::php_array_map);
        // registry.register_function(b"array_merge_recursive", array::php_array_merge_recursive);
        // registry.register_function(b"array_multisort", array::php_array_multisort);
        // registry.register_function(b"array_pad", array::php_array_pad);
        // registry.register_function(b"array_pop", array::php_array_pop);
        // registry.register_function(b"__deka_array_product", array::php_deka_array_product);
        // registry.register_function(b"__deka_array_reduce", array::php_deka_array_reduce);
        // registry.register_function(b"__deka_array_replace", array::php_deka_array_replace);
        // registry.register_function(
        //     b"__deka_array_replace_recursive",
        //     array::php_deka_array_replace_recursive,
        // );
        // registry.register_function(b"__deka_array_reverse", array::php_deka_array_reverse);
        // registry.register_function(b"__deka_array_search", array::php_deka_array_search);
        // registry.register_function(b"__deka_array_shift", array::php_deka_array_shift);
        // registry.register_function(b"__deka_array_slice", array::php_deka_array_slice);
        // registry.register_function(b"__deka_array_splice", array::php_deka_array_splice);
        // registry.register_function(b"array_product", array::php_array_product);
        // registry.register_function(b"array_reduce", array::php_array_reduce);
        // registry.register_function(b"array_replace", array::php_array_replace);
        // registry.register_function(b"array_replace_recursive", array::php_array_replace_recursive);
        // registry.register_function(b"array_reverse", array::php_array_reverse);
        // registry.register_function(b"array_search", array::php_array_search);
        // registry.register_function(b"array_shift", array::php_array_shift);
        // registry.register_function(b"array_slice", array::php_array_slice);
        // registry.register_function(b"array_splice", array::php_array_splice);
        // registry.register_function(b"__deka_array_sum", array::php_deka_array_sum);
        // registry.register_function(b"__deka_array_rand", array::php_deka_array_rand);
        // registry.register_function(b"__deka_array_udiff", array::php_deka_array_udiff);
        // registry.register_function(
        //     b"__deka_array_udiff_assoc",
        //     array::php_deka_array_udiff_assoc,
        // );
        // registry.register_function(
        //     b"__deka_array_udiff_uassoc",
        //     array::php_deka_array_udiff_uassoc,
        // );
        // registry.register_function(b"__deka_array_uintersect", array::php_deka_array_uintersect);
        // registry.register_function(
        //     b"__deka_array_uintersect_assoc",
        //     array::php_deka_array_uintersect_assoc,
        // );
        // registry.register_function(
        //     b"__deka_array_uintersect_uassoc",
        //     array::php_deka_array_uintersect_uassoc,
        // );
        // registry.register_function(b"__deka_array_unique", array::php_deka_array_unique);
        // registry.register_function(b"__deka_array_walk", array::php_deka_array_walk);
        // registry.register_function(
        //     b"__deka_array_walk_recursive",
        //     array::php_deka_array_walk_recursive,
        // );
        // registry.register_function(b"__deka_arsort", array::php_deka_arsort);
        // registry.register_function(b"__deka_asort", array::php_deka_asort);
        // registry.register_function(b"__deka_compact", array::php_deka_compact);
        // registry.register_function(b"__deka_extract", array::php_deka_extract);
        registry.register_function(b"__deka_array_cursor", array::php_deka_array_cursor);
        // registry.register_function(b"__deka_key", array::php_deka_key);
        // registry.register_function(b"__deka_key_exists", array::php_deka_key_exists);
        // registry.register_function(b"__deka_krsort", array::php_deka_krsort);
        // registry.register_function(b"__deka_natcasesort", array::php_deka_natcasesort);
        // registry.register_function(b"__deka_natsort", array::php_deka_natsort);
        // registry.register_function(b"__deka_pos", array::php_deka_pos);
        // registry.register_function(b"__deka_prev", array::php_deka_prev);
        // registry.register_function(b"__deka_range", array::php_deka_range);
        // registry.register_function(b"__deka_rsort", array::php_deka_rsort);
        // registry.register_function(b"__deka_shuffle", array::php_deka_shuffle);
        // registry.register_function(b"__deka_sort", array::php_deka_sort);
        // registry.register_function(b"__deka_uasort", array::php_deka_uasort);
        // registry.register_function(b"__deka_uksort", array::php_deka_uksort);
        // registry.register_function(b"__deka_usort", array::php_deka_usort);
        // registry.register_function(b"__deka_ksort", array::php_deka_ksort);
        // registry.register_function(b"__deka_current", array::php_deka_current);
        // registry.register_function(b"__deka_next", array::php_deka_next);
        // registry.register_function(b"__deka_reset", array::php_deka_reset);
        // registry.register_function(b"__deka_end", array::php_deka_end);
        // registry.register_function(b"array_sum", array::php_array_sum);
        // registry.register_function(b"array_rand", array::php_array_rand);
        // registry.register_function(b"array_udiff", array::php_array_udiff);
        // registry.register_function(b"array_udiff_assoc", array::php_array_udiff_assoc);
        // registry.register_function(b"array_udiff_uassoc", array::php_array_udiff_uassoc);
        // registry.register_function(b"array_uintersect", array::php_array_uintersect);
        // registry.register_function(b"array_uintersect_assoc", array::php_array_uintersect_assoc);
        // registry.register_function(b"array_uintersect_uassoc", array::php_array_uintersect_uassoc);
        // registry.register_function(b"array_unique", array::php_array_unique);
        // registry.register_function(b"array_walk", array::php_array_walk);
        // registry.register_function(b"array_walk_recursive", array::php_array_walk_recursive);
        // registry.register_function(b"arsort", array::php_arsort);
        // registry.register_function(b"asort", array::php_asort);
        // registry.register_function(b"compact", array::php_compact);
        // registry.register_function(b"extract", array::php_extract);
        // registry.register_function(b"key", array::php_key);
        // registry.register_function(b"key_exists", array::php_key_exists);
        // registry.register_function(b"krsort", array::php_krsort);
        // registry.register_function(b"natcasesort", array::php_natcasesort);
        // registry.register_function(b"natsort", array::php_natsort);
        // registry.register_function(b"pos", array::php_pos);
        // registry.register_function(b"prev", array::php_prev);
        // registry.register_function(b"range", array::php_range);
        // registry.register_function(b"rsort", array::php_rsort);
        // registry.register_function(b"shuffle", array::php_shuffle);
        // registry.register_function(b"sizeof", array::php_sizeof);
        // registry.register_function(b"sort", array::php_sort);
        // registry.register_function(b"uasort", array::php_uasort);
        // registry.register_function(b"uksort", array::php_uksort);
        // registry.register_function(b"usort", array::php_usort);
        // registry.register_function(b"array_keys", array::php_array_keys);
        // registry.register_function(b"array_values", array::php_array_values);
        registry.register_function(b"in_array", array::php_in_array);
        // registry.register_function(b"ksort", array::php_ksort);
        // registry.register_function(b"array_unshift", array::php_array_unshift);
        // registry.register_function(b"array_push", array::php_array_push);
        // registry.register_function(b"current", array::php_current);
        // registry.register_function(b"next", array::php_next);
        // registry.register_function(b"reset", array::php_reset);
        // registry.register_function(b"end", array::php_end);
        registry.register_function(b"array_key_exists", array::php_array_key_exists);
        registry.register_function(b"count", array::php_count);

        // Variable functions
        registry.register_function(b"__deka_symbol_get", variable::php_deka_symbol_get);
        registry.register_function(b"__deka_symbol_set", variable::php_deka_symbol_set);
        registry.register_function(b"__deka_symbol_exists", variable::php_deka_symbol_exists);
        registry.register_function(b"__deka_object_set", variable::php_deka_object_set);
        registry.register_function(b"__phpx_object_new", variable::php_phpx_object_new);
        registry.register_function(
            b"__phpx_object_to_stdclass",
            variable::php_phpx_object_to_stdclass,
        );
        registry.register_function(b"__phpx_struct_new", variable::php_phpx_struct_new);
        registry.register_function(b"__phpx_struct_set", variable::php_phpx_struct_set);
        registry.register_function(b"__deka_wasm_call", wasm::php_deka_wasm_call);
        registry.register_function(b"var_dump", variable::php_var_dump);
        registry.register_function(b"print_r", variable::php_print_r);
        registry.register_function(b"is_string", variable::php_is_string);
        registry.register_function(b"is_int", variable::php_is_int);
        registry.register_function(b"is_array", variable::php_is_array);
        registry.register_function(b"is_bool", variable::php_is_bool);
        registry.register_function(b"is_null", variable::php_is_null);
        registry.register_function(b"is_object", variable::php_is_object);
        registry.register_function(b"is_float", variable::php_is_float);
        registry.register_function(b"is_numeric", variable::php_is_numeric);
        registry.register_function(b"is_scalar", variable::php_is_scalar);
        registry.register_function(b"define", variable::php_define);
        registry.register_function(b"defined", variable::php_defined);
        registry.register_function(b"constant", variable::php_constant);
        registry.register_function(b"gettype", variable::php_gettype);
        registry.register_function(b"var_export", variable::php_var_export);
        registry.register_function(b"getenv", variable::php_getenv);
        registry.register_function(b"putenv", variable::php_putenv);
        registry.register_function(b"getopt", variable::php_getopt);
        registry.register_function(b"ini_get", variable::php_ini_get);
        registry.register_function(b"ini_set", variable::php_ini_set);
        registry.register_function(b"error_reporting", variable::php_error_reporting);
        registry.register_function(b"error_get_last", variable::php_error_get_last);

        // HTTP functions
        registry.register_function(b"header", http::php_header);
        registry.register_function(b"headers_sent", http::php_headers_sent);
        registry.register_function(b"header_remove", http::php_header_remove);

        // URL functions
        registry.register_function(b"urlencode", url::php_urlencode);
        registry.register_function(b"urldecode", url::php_urldecode);
        registry.register_function(b"rawurlencode", url::php_rawurlencode);
        registry.register_function(b"rawurldecode", url::php_rawurldecode);
        registry.register_function(b"base64_encode", url::php_base64_encode);
        registry.register_function(b"base64_decode", url::php_base64_decode);
        registry.register_function(b"parse_url", url::php_parse_url);
        registry.register_function(b"http_build_query", url::php_http_build_query);
        registry.register_function(b"get_headers", url::php_get_headers);
        registry.register_function(b"get_meta_tags", url::php_get_meta_tags);

        // Math functions
        registry.register_function(b"abs", math::php_abs);
        registry.register_function(b"max", math::php_max);
        registry.register_function(b"min", math::php_min);
        registry.register_function(b"pi", math::php_pi);
        registry.register_function(b"pow", math::php_pow);
        registry.register_function(b"fpow", math::php_fpow);
        registry.register_function(b"srand", math::php_srand);
        registry.register_function(b"rand", math::php_rand);
        registry.register_function(b"getrandmax", math::php_getrandmax);
        registry.register_function(b"ceil", math::php_ceil);
        registry.register_function(b"floor", math::php_floor);
        registry.register_function(b"round", math::php_round);
        registry.register_function(b"sin", math::php_sin);
        registry.register_function(b"sinh", math::php_sinh);
        registry.register_function(b"cos", math::php_cos);
        registry.register_function(b"cosh", math::php_cosh);
        registry.register_function(b"tan", math::php_tan);
        registry.register_function(b"tanh", math::php_tanh);
        registry.register_function(b"asin", math::php_asin);
        registry.register_function(b"asinh", math::php_asinh);
        registry.register_function(b"acos", math::php_acos);
        registry.register_function(b"acosh", math::php_acosh);
        registry.register_function(b"atan", math::php_atan);
        registry.register_function(b"atanh", math::php_atanh);
        registry.register_function(b"atan2", math::php_atan2);
        registry.register_function(b"deg2rad", math::php_deg2rad);
        registry.register_function(b"rad2deg", math::php_rad2deg);
        registry.register_function(b"exp", math::php_exp);
        registry.register_function(b"expm1", math::php_expm1);
        registry.register_function(b"log", math::php_log);
        registry.register_function(b"log10", math::php_log10);
        registry.register_function(b"log1p", math::php_log1p);
        registry.register_function(b"sqrt", math::php_sqrt);
        registry.register_function(b"fdiv", math::php_fdiv);
        registry.register_function(b"intdiv", math::php_intdiv);
        registry.register_function(b"fmod", math::php_fmod);
        registry.register_function(b"hypot", math::php_hypot);
        registry.register_function(b"is_finite", math::php_is_finite);
        registry.register_function(b"is_infinite", math::php_is_infinite);
        registry.register_function(b"is_nan", math::php_is_nan);
        registry.register_function(b"base_convert", math::php_base_convert);
        registry.register_function(b"decbin", math::php_decbin);
        registry.register_function(b"dechex", math::php_dechex);
        registry.register_function(b"decoct", math::php_decoct);
        registry.register_function(b"bindec", math::php_bindec);
        registry.register_function(b"hexdec", math::php_hexdec);
        registry.register_function(b"octdec", math::php_octdec);

        // BCMath functions
        registry.register_function(b"bcadd", bcmath::bcadd);
        registry.register_function(b"bcsub", bcmath::bcsub);
        registry.register_function(b"bcmul", bcmath::bcmul);
        registry.register_function(b"bcdiv", bcmath::bcdiv);

        // Class functions
        registry.register_function(b"get_object_vars", class::php_get_object_vars);
        registry.register_function(b"get_class", class::php_get_class);
        registry.register_function(b"get_parent_class", class::php_get_parent_class);
        registry.register_function(b"is_subclass_of", class::php_is_subclass_of);
        registry.register_function(b"is_a", class::php_is_a);
        registry.register_function(b"class_exists", class::php_class_exists);
        registry.register_function(b"class_alias", class::php_class_alias);
        registry.register_function(b"interface_exists", class::php_interface_exists);
        registry.register_function(b"trait_exists", class::php_trait_exists);
        registry.register_function(b"method_exists", class::php_method_exists);
        registry.register_function(b"property_exists", class::php_property_exists);
        registry.register_function(b"get_class_methods", class::php_get_class_methods);
        registry.register_function(b"get_class_vars", class::php_get_class_vars);
        registry.register_function(b"get_called_class", class::php_get_called_class);

        // PCRE functions
        registry.register_function(b"preg_match", pcre::preg_match);
        registry.register_function(b"preg_replace", pcre::preg_replace);
        registry.register_function(b"preg_split", pcre::preg_split);
        registry.register_function(b"preg_quote", pcre::preg_quote);

        // Function handling functions
        registry.register_function(b"func_get_args", function::php_func_get_args);
        registry.register_function(b"func_num_args", function::php_func_num_args);
        registry.register_function(b"func_get_arg", function::php_func_get_arg);
        registry.register_function(b"function_exists", function::php_function_exists);
        registry.register_function(b"is_callable", function::php_is_callable);
        registry.register_function(b"call_user_func", function::php_call_user_func);
        registry.register_function(b"call_user_func_array", function::php_call_user_func_array);
        registry.register_function(b"extension_loaded", function::php_extension_loaded);
        registry.register_function(b"spl_autoload_register", spl::php_spl_autoload_register);
        registry.register_function(b"spl_object_hash", spl::php_spl_object_hash);
        registry.register_function(b"assert", function::php_assert);

        // Filesystem functions - File I/O
        registry.register_function(b"fopen", filesystem::php_fopen);
        registry.register_function(b"fclose", filesystem::php_fclose);
        registry.register_function(b"fread", filesystem::php_fread);
        registry.register_function(b"fwrite", filesystem::php_fwrite);
        registry.register_function(b"fputs", filesystem::php_fputs);
        registry.register_function(b"fgets", filesystem::php_fgets);
        registry.register_function(b"fgetc", filesystem::php_fgetc);
        registry.register_function(b"stream_get_contents", filesystem::php_stream_get_contents);
        registry.register_function(b"fseek", filesystem::php_fseek);
        registry.register_function(b"ftell", filesystem::php_ftell);
        registry.register_function(b"rewind", filesystem::php_rewind);
        registry.register_function(b"feof", filesystem::php_feof);
        registry.register_function(b"fflush", filesystem::php_fflush);

        // Filesystem functions - File content
        registry.register_function(b"file_get_contents", filesystem::php_file_get_contents);
        registry.register_function(b"file_put_contents", filesystem::php_file_put_contents);
        registry.register_function(b"file", filesystem::php_file);

        // Filesystem functions - File information
        registry.register_function(b"file_exists", filesystem::php_file_exists);
        registry.register_function(b"is_file", filesystem::php_is_file);
        registry.register_function(b"is_dir", filesystem::php_is_dir);
        registry.register_function(b"is_link", filesystem::php_is_link);
        registry.register_function(b"is_readable", filesystem::php_is_readable);
        registry.register_function(b"is_writable", filesystem::php_is_writable);
        registry.register_function(b"is_writeable", filesystem::php_is_writable); // Alias
        registry.register_function(b"is_executable", filesystem::php_is_executable);

        // Filesystem functions - File metadata
        registry.register_function(b"filesize", filesystem::php_filesize);
        registry.register_function(b"filemtime", filesystem::php_filemtime);
        registry.register_function(b"filectime", filesystem::php_filectime);
        registry.register_function(b"fileatime", filesystem::php_fileatime);
        registry.register_function(b"fileperms", filesystem::php_fileperms);
        registry.register_function(b"fileowner", filesystem::php_fileowner);
        registry.register_function(b"filegroup", filesystem::php_filegroup);
        registry.register_function(b"stat", filesystem::php_stat);
        registry.register_function(b"lstat", filesystem::php_lstat);

        // Filesystem functions - File operations
        registry.register_function(b"unlink", filesystem::php_unlink);
        registry.register_function(b"rename", filesystem::php_rename);
        registry.register_function(b"copy", filesystem::php_copy);
        registry.register_function(b"touch", filesystem::php_touch);
        registry.register_function(b"chmod", filesystem::php_chmod);
        registry.register_function(b"readlink", filesystem::php_readlink);
        registry.register_function(b"realpath", filesystem::php_realpath);

        // Filesystem functions - Directory operations
        registry.register_function(b"mkdir", filesystem::php_mkdir);
        registry.register_function(b"rmdir", filesystem::php_rmdir);
        registry.register_function(b"scandir", filesystem::php_scandir);
        registry.register_function(b"getcwd", filesystem::php_getcwd);
        registry.register_function(b"chdir", filesystem::php_chdir);

        // Filesystem functions - Path operations
        registry.register_function(b"basename", filesystem::php_basename);
        registry.register_function(b"dirname", filesystem::php_dirname);

        // Filesystem functions - Temporary files
        registry.register_function(b"sys_get_temp_dir", filesystem::php_sys_get_temp_dir);
        registry.register_function(b"tmpfile", filesystem::php_tmpfile);
        registry.register_function(b"tempnam", filesystem::php_tempnam);

        // Filesystem functions - Disk space
        registry.register_function(b"disk_free_space", filesystem::php_disk_free_space);
        registry.register_function(b"disk_total_space", filesystem::php_disk_total_space);

        // Execution functions
        registry.register_function(b"escapeshellarg", exec::php_escapeshellarg);
        registry.register_function(b"escapeshellcmd", exec::php_escapeshellcmd);
        registry.register_function(b"exec", exec::php_exec);
        registry.register_function(b"passthru", exec::php_passthru);
        registry.register_function(b"shell_exec", exec::php_shell_exec);
        registry.register_function(b"system", exec::php_system);
        registry.register_function(b"proc_open", exec::php_proc_open);
        registry.register_function(b"proc_close", exec::php_proc_close);
        registry.register_function(b"proc_terminate", exec::php_proc_terminate);
        registry.register_function(b"proc_nice", exec::php_proc_nice);
        registry.register_function(b"proc_get_status", exec::php_proc_get_status);
        registry.register_function(b"set_time_limit", exec::php_set_time_limit);

        // ========================================
        // CORE PHP INTERFACES
        // ========================================

        // Stringable interface (PHP 8.0+)
        registry.register_class(NativeClassDef {
            name: b"Stringable".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Throwable interface (base for all exceptions/errors, extends Stringable)
        registry.register_class(NativeClassDef {
            name: b"Throwable".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![b"Stringable".to_vec()],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Traversable interface (root iterator interface)
        registry.register_class(NativeClassDef {
            name: b"Traversable".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Iterator interface
        registry.register_class(NativeClassDef {
            name: b"Iterator".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![b"Traversable".to_vec()],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // IteratorAggregate interface
        registry.register_class(NativeClassDef {
            name: b"IteratorAggregate".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![b"Traversable".to_vec()],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Countable interface
        registry.register_class(NativeClassDef {
            name: b"Countable".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // ArrayAccess interface
        registry.register_class(NativeClassDef {
            name: b"ArrayAccess".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Serializable interface (deprecated since PHP 8.1)
        registry.register_class(NativeClassDef {
            name: b"Serializable".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // Attribute class (PHP 8.0+)
        let mut attribute_constants = HashMap::new();
        attribute_constants.insert(b"TARGET_CLASS".to_vec(), (Val::Int(1), Visibility::Public));
        attribute_constants.insert(b"TARGET_METHOD".to_vec(), (Val::Int(2), Visibility::Public));
        attribute_constants.insert(
            b"TARGET_FUNCTION".to_vec(),
            (Val::Int(4), Visibility::Public),
        );
        attribute_constants.insert(
            b"TARGET_PROPERTY".to_vec(),
            (Val::Int(8), Visibility::Public),
        );
        attribute_constants.insert(
            b"TARGET_CLASS_CONSTANT".to_vec(),
            (Val::Int(16), Visibility::Public),
        );
        attribute_constants.insert(
            b"TARGET_PARAMETER".to_vec(),
            (Val::Int(32), Visibility::Public),
        );
        attribute_constants.insert(b"TARGET_ALL".to_vec(), (Val::Int(63), Visibility::Public));
        attribute_constants.insert(
            b"IS_REPEATABLE".to_vec(),
            (Val::Int(64), Visibility::Public),
        );
        registry.register_class(NativeClassDef {
            name: b"Attribute".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: attribute_constants,
            constructor: None,
        });

        // UnitEnum interface (PHP 8.1+)
        registry.register_class(NativeClassDef {
            name: b"UnitEnum".to_vec(),
            parent: None,
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // BackedEnum interface (PHP 8.1+)
        registry.register_class(NativeClassDef {
            name: b"BackedEnum".to_vec(),
            parent: Some(b"UnitEnum".to_vec()),
            is_interface: true,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // ========================================
        // INTERNAL CLASSES
        // ========================================

        // Closure class (final)
        let mut closure_methods = HashMap::new();
        closure_methods.insert(
            b"bind".to_vec(),
            NativeMethodEntry {
                handler: class::closure_bind,
                visibility: Visibility::Public,
                is_static: true,
            },
        );
        closure_methods.insert(
            b"bindTo".to_vec(),
            NativeMethodEntry {
                handler: class::closure_bind_to,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        closure_methods.insert(
            b"call".to_vec(),
            NativeMethodEntry {
                handler: class::closure_call,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        closure_methods.insert(
            b"fromCallable".to_vec(),
            NativeMethodEntry {
                handler: class::closure_from_callable,
                visibility: Visibility::Public,
                is_static: true,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"Closure".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: closure_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // stdClass - empty class for generic objects
        registry.register_class(NativeClassDef {
            name: b"stdClass".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionClass
        let mut reflection_class_methods = HashMap::new();
        reflection_class_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_class_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_class_methods.insert(
            b"getAttributes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_get_attributes,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionClass".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_class_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionAttribute
        let mut reflection_attr_methods = HashMap::new();
        reflection_attr_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_attribute_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_attr_methods.insert(
            b"getArguments".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_attribute_get_arguments,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_attr_methods.insert(
            b"newInstance".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_attribute_new_instance,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionAttribute".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_attr_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionMethod
        let mut reflection_method_methods = HashMap::new();
        reflection_method_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_method_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_method_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_method_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_method_methods.insert(
            b"getAttributes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_method_get_attributes,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_method_methods.insert(
            b"getParameters".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_method_get_parameters,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_method_methods.insert(
            b"getReturnType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_method_get_return_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionMethod".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_method_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionProperty
        let mut reflection_prop_methods = HashMap::new();
        reflection_prop_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_property_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_prop_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_property_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_prop_methods.insert(
            b"getAttributes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_property_get_attributes,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_prop_methods.insert(
            b"hasType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_property_has_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_prop_methods.insert(
            b"getType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_property_get_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionProperty".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_prop_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionClassConstant
        let mut reflection_const_methods = HashMap::new();
        reflection_const_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_const_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_const_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_const_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_const_methods.insert(
            b"getAttributes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_class_const_get_attributes,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionClassConstant".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_const_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionParameter
        let mut reflection_param_methods = HashMap::new();
        reflection_param_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_parameter_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_param_methods.insert(
            b"hasType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_parameter_has_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_param_methods.insert(
            b"getType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_parameter_get_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_param_methods.insert(
            b"isVariadic".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_parameter_is_variadic,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_param_methods.insert(
            b"isPassedByReference".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_parameter_is_passed_by_reference,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionParameter".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_param_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionNamedType
        let mut reflection_named_methods = HashMap::new();
        reflection_named_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_named_type_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_named_methods.insert(
            b"allowsNull".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_named_type_allows_null,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_named_methods.insert(
            b"isBuiltin".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_named_type_is_builtin,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionNamedType".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_named_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionUnionType
        let mut reflection_union_methods = HashMap::new();
        reflection_union_methods.insert(
            b"getTypes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_union_type_get_types,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_union_methods.insert(
            b"allowsNull".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_union_type_allows_null,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionUnionType".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_union_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionIntersectionType
        let mut reflection_intersection_methods = HashMap::new();
        reflection_intersection_methods.insert(
            b"getTypes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_intersection_type_get_types,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_intersection_methods.insert(
            b"allowsNull".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_intersection_type_allows_null,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionIntersectionType".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_intersection_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // ReflectionFunction
        let mut reflection_function_methods = HashMap::new();
        reflection_function_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_function_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_function_methods.insert(
            b"getName".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_function_get_name,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_function_methods.insert(
            b"getAttributes".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_function_get_attributes,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_function_methods.insert(
            b"getParameters".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_function_get_parameters,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        reflection_function_methods.insert(
            b"getReturnType".to_vec(),
            NativeMethodEntry {
                handler: reflection::reflection_function_get_return_type,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"ReflectionFunction".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: reflection_function_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // Generator class (final, implements Iterator)
        let mut generator_methods = HashMap::new();
        generator_methods.insert(
            b"current".to_vec(),
            NativeMethodEntry {
                handler: class::generator_current,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"key".to_vec(),
            NativeMethodEntry {
                handler: class::generator_key,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"next".to_vec(),
            NativeMethodEntry {
                handler: class::generator_next,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"rewind".to_vec(),
            NativeMethodEntry {
                handler: class::generator_rewind,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"valid".to_vec(),
            NativeMethodEntry {
                handler: class::generator_valid,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"send".to_vec(),
            NativeMethodEntry {
                handler: class::generator_send,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"throw".to_vec(),
            NativeMethodEntry {
                handler: class::generator_throw,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        generator_methods.insert(
            b"getReturn".to_vec(),
            NativeMethodEntry {
                handler: class::generator_get_return,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"Generator".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![b"Iterator".to_vec()],
            methods: generator_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // Fiber class (PHP 8.1+)
        let mut fiber_methods = HashMap::new();
        fiber_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"start".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_start,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"resume".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_resume,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"suspend".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_suspend,
                visibility: Visibility::Public,
                is_static: true,
            },
        );
        fiber_methods.insert(
            b"throw".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_throw,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"isStarted".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_is_started,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"isSuspended".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_is_suspended,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"isRunning".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_is_running,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"isTerminated".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_is_terminated,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"getReturn".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_get_return,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        fiber_methods.insert(
            b"getCurrent".to_vec(),
            NativeMethodEntry {
                handler: class::fiber_get_current,
                visibility: Visibility::Public,
                is_static: true,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"Fiber".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: fiber_methods,
            constants: HashMap::new(),
            constructor: Some(class::fiber_construct),
        });

        // WeakReference class (PHP 7.4+)
        let mut weakref_methods = HashMap::new();
        weakref_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: class::weak_reference_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakref_methods.insert(
            b"create".to_vec(),
            NativeMethodEntry {
                handler: class::weak_reference_create,
                visibility: Visibility::Public,
                is_static: true,
            },
        );
        weakref_methods.insert(
            b"get".to_vec(),
            NativeMethodEntry {
                handler: class::weak_reference_get,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"WeakReference".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: weakref_methods,
            constants: HashMap::new(),
            constructor: Some(class::weak_reference_construct),
        });

        // WeakMap class (PHP 8.0+, implements ArrayAccess, Countable, IteratorAggregate)
        let mut weakmap_methods = HashMap::new();
        weakmap_methods.insert(
            b"offsetExists".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_offset_exists,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakmap_methods.insert(
            b"offsetGet".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_offset_get,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakmap_methods.insert(
            b"offsetSet".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_offset_set,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakmap_methods.insert(
            b"offsetUnset".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_offset_unset,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakmap_methods.insert(
            b"count".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_count,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        weakmap_methods.insert(
            b"getIterator".to_vec(),
            NativeMethodEntry {
                handler: class::weak_map_get_iterator,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"WeakMap".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![
                b"ArrayAccess".to_vec(),
                b"Countable".to_vec(),
                b"IteratorAggregate".to_vec(),
            ],
            methods: weakmap_methods,
            constants: HashMap::new(),
            constructor: None,
        });

        // SensitiveParameterValue class (PHP 8.2+)
        let mut sensitive_methods = HashMap::new();
        sensitive_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: class::sensitive_parameter_value_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        sensitive_methods.insert(
            b"getValue".to_vec(),
            NativeMethodEntry {
                handler: class::sensitive_parameter_value_get_value,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        sensitive_methods.insert(
            b"__debugInfo".to_vec(),
            NativeMethodEntry {
                handler: class::sensitive_parameter_value_debug_info,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"SensitiveParameterValue".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: sensitive_methods,
            constants: HashMap::new(),
            constructor: Some(class::sensitive_parameter_value_construct),
        });

        // __PHP_Incomplete_Class (used during unserialization)
        registry.register_class(NativeClassDef {
            name: b"__PHP_Incomplete_Class".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: None,
        });

        // ========================================
        // EXCEPTION HIERARCHY
        // ========================================

        // Exception class
        let mut exception_methods = HashMap::new();
        exception_methods.insert(
            b"__construct".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_construct,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getMessage".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_message,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getCode".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_code,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getFile".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_file,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getLine".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_line,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getTrace".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_trace,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getTraceAsString".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_trace_as_string,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"getPrevious".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_get_previous,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        exception_methods.insert(
            b"__toString".to_vec(),
            NativeMethodEntry {
                handler: exception::exception_to_string,
                visibility: Visibility::Public,
                is_static: false,
            },
        );
        registry.register_class(NativeClassDef {
            name: b"Exception".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![b"Throwable".to_vec()],
            methods: exception_methods.clone(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // RuntimeException
        registry.register_class(NativeClassDef {
            name: b"RuntimeException".to_vec(),
            parent: Some(b"Exception".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // LogicException
        registry.register_class(NativeClassDef {
            name: b"LogicException".to_vec(),
            parent: Some(b"Exception".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // Error class (PHP 7+)
        registry.register_class(NativeClassDef {
            name: b"Error".to_vec(),
            parent: None,
            is_interface: false,
            is_trait: false,
            interfaces: vec![b"Throwable".to_vec()],
            methods: exception_methods.clone(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // TypeError
        registry.register_class(NativeClassDef {
            name: b"TypeError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // ArithmeticError
        registry.register_class(NativeClassDef {
            name: b"ArithmeticError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // DivisionByZeroError
        registry.register_class(NativeClassDef {
            name: b"DivisionByZeroError".to_vec(),
            parent: Some(b"ArithmeticError".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // ParseError
        registry.register_class(NativeClassDef {
            name: b"ParseError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // AssertionError
        registry.register_class(NativeClassDef {
            name: b"AssertionError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // CompileError (PHP 7.3+)
        registry.register_class(NativeClassDef {
            name: b"CompileError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // ValueError (PHP 8.0+)
        registry.register_class(NativeClassDef {
            name: b"ValueError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // UnhandledMatchError (PHP 8.0+)
        registry.register_class(NativeClassDef {
            name: b"UnhandledMatchError".to_vec(),
            parent: Some(b"Error".to_vec()),
            is_interface: false,
            is_trait: false,
            interfaces: vec![],
            methods: HashMap::new(),
            constants: HashMap::new(),
            constructor: Some(exception::exception_construct),
        });

        // Output Control functions
        registry.register_function(b"ob_start", output_control::php_ob_start);
        registry.register_function(b"ob_end_clean", output_control::php_ob_end_clean);
        registry.register_function(b"ob_end_flush", output_control::php_ob_end_flush);
        registry.register_function(b"ob_clean", output_control::php_ob_clean);
        registry.register_function(b"ob_flush", output_control::php_ob_flush);
        registry.register_function(b"ob_get_contents", output_control::php_ob_get_contents);
        registry.register_function(b"ob_get_clean", output_control::php_ob_get_clean);
        registry.register_function(b"ob_get_flush", output_control::php_ob_get_flush);
        registry.register_function(b"ob_get_length", output_control::php_ob_get_length);
        registry.register_function(b"ob_get_level", output_control::php_ob_get_level);
        registry.register_function(b"ob_get_status", output_control::php_ob_get_status);
        registry.register_function(b"ob_implicit_flush", output_control::php_ob_implicit_flush);
        registry.register_function(b"ob_list_handlers", output_control::php_ob_list_handlers);
        registry.register_function(
            b"output_add_rewrite_var",
            output_control::php_output_add_rewrite_var,
        );
        registry.register_function(
            b"output_reset_rewrite_vars",
            output_control::php_output_reset_rewrite_vars,
        );

        // Register core string constants
        registry.register_constant(b"STR_PAD_LEFT", Val::Int(0));
        registry.register_constant(b"STR_PAD_RIGHT", Val::Int(1));
        registry.register_constant(b"STR_PAD_BOTH", Val::Int(2));
        registry.register_constant(b"HTML_SPECIALCHARS", Val::Int(string::HTML_SPECIALCHARS));
        registry.register_constant(b"HTML_ENTITIES", Val::Int(string::HTML_ENTITIES));
        registry.register_constant(b"ENT_NOQUOTES", Val::Int(string::ENT_NOQUOTES));
        registry.register_constant(b"ENT_COMPAT", Val::Int(string::ENT_COMPAT));
        registry.register_constant(b"ENT_QUOTES", Val::Int(string::ENT_QUOTES));
        registry.register_constant(b"ENT_SUBSTITUTE", Val::Int(string::ENT_SUBSTITUTE));
        registry.register_constant(b"ENT_HTML401", Val::Int(string::ENT_HTML401));
        registry.register_constant(b"ENT_XML1", Val::Int(string::ENT_XML1));
        registry.register_constant(b"ENT_XHTML", Val::Int(string::ENT_XHTML));
        registry.register_constant(b"ENT_HTML5", Val::Int(string::ENT_HTML5));

        // Register locale category constants
        #[cfg(unix)]
        {
            registry.register_constant(b"LC_ALL", Val::Int(libc::LC_ALL as i64));
            registry.register_constant(b"LC_COLLATE", Val::Int(libc::LC_COLLATE as i64));
            registry.register_constant(b"LC_CTYPE", Val::Int(libc::LC_CTYPE as i64));
            registry.register_constant(b"LC_MONETARY", Val::Int(libc::LC_MONETARY as i64));
            registry.register_constant(b"LC_NUMERIC", Val::Int(libc::LC_NUMERIC as i64));
            registry.register_constant(b"LC_TIME", Val::Int(libc::LC_TIME as i64));
            #[cfg(any(
                target_os = "linux",
                target_os = "android",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "dragonfly"
            ))]
            registry.register_constant(b"LC_MESSAGES", Val::Int(libc::LC_MESSAGES as i64));
        }
        #[cfg(target_arch = "wasm32")]
        {
            registry.register_constant(b"LC_ALL", Val::Int(0));
            registry.register_constant(b"LC_COLLATE", Val::Int(0));
            registry.register_constant(b"LC_CTYPE", Val::Int(0));
            registry.register_constant(b"LC_MONETARY", Val::Int(0));
            registry.register_constant(b"LC_NUMERIC", Val::Int(0));
            registry.register_constant(b"LC_TIME", Val::Int(0));
        }

        // Register nl_langinfo constants
        #[cfg(unix)]
        {
            registry.register_constant(b"CODESET", Val::Int(libc::CODESET as i64));
            registry.register_constant(b"D_T_FMT", Val::Int(libc::D_T_FMT as i64));
            registry.register_constant(b"D_FMT", Val::Int(libc::D_FMT as i64));
            registry.register_constant(b"T_FMT", Val::Int(libc::T_FMT as i64));
            registry.register_constant(b"T_FMT_AMPM", Val::Int(libc::T_FMT_AMPM as i64));

            registry.register_constant(b"DAY_1", Val::Int(libc::DAY_1 as i64));
            registry.register_constant(b"DAY_2", Val::Int(libc::DAY_2 as i64));
            registry.register_constant(b"DAY_3", Val::Int(libc::DAY_3 as i64));
            registry.register_constant(b"DAY_4", Val::Int(libc::DAY_4 as i64));
            registry.register_constant(b"DAY_5", Val::Int(libc::DAY_5 as i64));
            registry.register_constant(b"DAY_6", Val::Int(libc::DAY_6 as i64));
            registry.register_constant(b"DAY_7", Val::Int(libc::DAY_7 as i64));

            registry.register_constant(b"ABDAY_1", Val::Int(libc::ABDAY_1 as i64));
            registry.register_constant(b"ABDAY_2", Val::Int(libc::ABDAY_2 as i64));
            registry.register_constant(b"ABDAY_3", Val::Int(libc::ABDAY_3 as i64));
            registry.register_constant(b"ABDAY_4", Val::Int(libc::ABDAY_4 as i64));
            registry.register_constant(b"ABDAY_5", Val::Int(libc::ABDAY_5 as i64));
            registry.register_constant(b"ABDAY_6", Val::Int(libc::ABDAY_6 as i64));
            registry.register_constant(b"ABDAY_7", Val::Int(libc::ABDAY_7 as i64));

            registry.register_constant(b"MON_1", Val::Int(libc::MON_1 as i64));
            registry.register_constant(b"MON_2", Val::Int(libc::MON_2 as i64));
            registry.register_constant(b"MON_3", Val::Int(libc::MON_3 as i64));
            registry.register_constant(b"MON_4", Val::Int(libc::MON_4 as i64));
            registry.register_constant(b"MON_5", Val::Int(libc::MON_5 as i64));
            registry.register_constant(b"MON_6", Val::Int(libc::MON_6 as i64));
            registry.register_constant(b"MON_7", Val::Int(libc::MON_7 as i64));
            registry.register_constant(b"MON_8", Val::Int(libc::MON_8 as i64));
            registry.register_constant(b"MON_9", Val::Int(libc::MON_9 as i64));
            registry.register_constant(b"MON_10", Val::Int(libc::MON_10 as i64));
            registry.register_constant(b"MON_11", Val::Int(libc::MON_11 as i64));
            registry.register_constant(b"MON_12", Val::Int(libc::MON_12 as i64));

            registry.register_constant(b"ABMON_1", Val::Int(libc::ABMON_1 as i64));
            registry.register_constant(b"ABMON_2", Val::Int(libc::ABMON_2 as i64));
            registry.register_constant(b"ABMON_3", Val::Int(libc::ABMON_3 as i64));
            registry.register_constant(b"ABMON_4", Val::Int(libc::ABMON_4 as i64));
            registry.register_constant(b"ABMON_5", Val::Int(libc::ABMON_5 as i64));
            registry.register_constant(b"ABMON_6", Val::Int(libc::ABMON_6 as i64));
            registry.register_constant(b"ABMON_7", Val::Int(libc::ABMON_7 as i64));
            registry.register_constant(b"ABMON_8", Val::Int(libc::ABMON_8 as i64));
            registry.register_constant(b"ABMON_9", Val::Int(libc::ABMON_9 as i64));
            registry.register_constant(b"ABMON_10", Val::Int(libc::ABMON_10 as i64));
            registry.register_constant(b"ABMON_11", Val::Int(libc::ABMON_11 as i64));
            registry.register_constant(b"ABMON_12", Val::Int(libc::ABMON_12 as i64));
        }
        #[cfg(target_arch = "wasm32")]
        {
            registry.register_constant(b"CODESET", Val::Int(0));
        }

        // Register URL constants
        registry.register_constant(b"PHP_URL_SCHEME", Val::Int(url::PHP_URL_SCHEME));
        registry.register_constant(b"PHP_URL_HOST", Val::Int(url::PHP_URL_HOST));
        registry.register_constant(b"PHP_URL_PORT", Val::Int(url::PHP_URL_PORT));
        registry.register_constant(b"PHP_URL_USER", Val::Int(url::PHP_URL_USER));
        registry.register_constant(b"PHP_URL_PASS", Val::Int(url::PHP_URL_PASS));
        registry.register_constant(b"PHP_URL_PATH", Val::Int(url::PHP_URL_PATH));
        registry.register_constant(b"PHP_URL_QUERY", Val::Int(url::PHP_URL_QUERY));
        registry.register_constant(b"PHP_URL_FRAGMENT", Val::Int(url::PHP_URL_FRAGMENT));
        registry.register_constant(b"PHP_QUERY_RFC1738", Val::Int(url::PHP_QUERY_RFC1738));
        registry.register_constant(b"PHP_QUERY_RFC3986", Val::Int(url::PHP_QUERY_RFC3986));

        // Register output control constants - Phase flags
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_START",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_START),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_WRITE",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_WRITE),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_FLUSH",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_FLUSH),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_CLEAN",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_CLEAN),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_FINAL",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_FINAL),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_CONT",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_CONT),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_END",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_END),
        );

        // Register output control constants - Control flags
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_CLEANABLE",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_CLEANABLE),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_FLUSHABLE",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_FLUSHABLE),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_REMOVABLE",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_REMOVABLE),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_STDFLAGS",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_STDFLAGS),
        );

        // Register output control constants - Status flags
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_STARTED",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_STARTED),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_DISABLED",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_DISABLED),
        );
        registry.register_constant(
            b"PHP_OUTPUT_HANDLER_PROCESSED",
            Val::Int(output_control::PHP_OUTPUT_HANDLER_PROCESSED),
        );

        ExtensionResult::Success
    }

    fn request_init(&self, ctx: &mut RequestContext) -> ExtensionResult {
        ctx.set_extension_data(CoreExtensionData::default());
        ExtensionResult::Success
    }

    fn request_shutdown(&self, _ctx: &mut RequestContext) -> ExtensionResult {
        ExtensionResult::Success
    }

    fn module_shutdown(&self) -> ExtensionResult {
        ExtensionResult::Success
    }
}
