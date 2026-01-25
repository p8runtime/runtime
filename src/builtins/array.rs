use crate::core::value::{ArrayKey, Handle, Val};
use crate::runtime::core_extension::CoreExtensionData;
use crate::vm::engine::VM;
use crate::vm::frame::ArgList;
use indexmap::IndexMap;
use std::rc::Rc;

pub fn php_array(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let mut map = IndexMap::new();
    for (idx, handle) in args.iter().enumerate() {
        map.insert(ArrayKey::Int(idx as i64), *handle);
    }
    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())))
}

pub fn php_count(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("count() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    let count = match &val.value {
        Val::Array(arr) => arr.map.len(),
        Val::Null => 0,
        Val::ConstArray(map) => map.len(),
        // In PHP, count() on non-array/non-Countable returns 1 (no strict mode for count)
        _ => 1,
    };

    Ok(vm.arena.alloc(Val::Int(count as i64)))
}

pub fn php_array_all(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_all() expects exactly 2 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_all() expects parameter 1 to be array".into()),
    };

    let callback = args[1];
    for (key, value_handle) in entries {
        let key_handle = match key {
            ArrayKey::Int(i) => vm.arena.alloc(Val::Int(i)),
            ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.into())),
        };
        let result = vm
            .call_callable(callback, smallvec::smallvec![value_handle, key_handle])
            .map_err(|e| format!("array_all(): {}", e))?;
        if !vm.arena.get(result).value.to_bool() {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_array_any(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_any() expects exactly 2 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_any() expects parameter 1 to be array".into()),
    };

    let callback = args[1];
    for (key, value_handle) in entries {
        let key_handle = match key {
            ArrayKey::Int(i) => vm.arena.alloc(Val::Int(i)),
            ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.into())),
        };
        let result = vm
            .call_callable(callback, smallvec::smallvec![value_handle, key_handle])
            .map_err(|e| format!("array_any(): {}", e))?;
        if vm.arena.get(result).value.to_bool() {
            return Ok(vm.arena.alloc(Val::Bool(true)));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_array_change_key_case(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("array_change_key_case() expects 1 or 2 parameters".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_change_key_case() expects parameter 1 to be array".into()),
    };

    let mode = if args.len() == 2 {
        vm.arena.get(args[1]).value.to_int()
    } else {
        0
    };

    let mut map = IndexMap::new();
    for (key, value_handle) in arr.map.iter() {
        let new_key = match key {
            ArrayKey::Int(i) => ArrayKey::Int(*i),
            ArrayKey::Str(s) => {
                let mut bytes = s.as_ref().to_vec();
                if mode == 1 {
                    for b in &mut bytes {
                        *b = b.to_ascii_uppercase();
                    }
                } else {
                    for b in &mut bytes {
                        *b = b.to_ascii_lowercase();
                    }
                }
                ArrayKey::Str(Rc::new(bytes))
            }
        };
        map.insert(new_key, *value_handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())))
}

pub fn php_array_chunk(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_chunk() expects 2 or 3 parameters".into());
    }

    let size = vm.check_builtin_param_int(args[1], 2, "array_chunk")?;
    if size <= 0 {
        return Err("array_chunk(): Size parameter expected to be greater than 0".into());
    }
    let preserve_keys = if args.len() == 3 {
        vm.arena.get(args[2]).value.to_bool()
    } else {
        false
    };
    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_chunk() expects parameter 1 to be array".into()),
    };

    let mut out = IndexMap::new();
    let mut chunk = IndexMap::new();
    let mut chunk_index = 0i64;
    let mut chunk_pos = 0i64;
    let mut chunk_len = 0i64;

    for (key, value_handle) in entries {
        let key_to_use = if preserve_keys {
            key
        } else {
            ArrayKey::Int(chunk_pos)
        };
        chunk.insert(key_to_use, value_handle);
        chunk_pos += 1;
        chunk_len += 1;
        if chunk_len == size {
            let chunk_handle = vm.arena.alloc(Val::Array(
                crate::core::value::ArrayData::from(chunk).into(),
            ));
            out.insert(ArrayKey::Int(chunk_index), chunk_handle);
            chunk_index += 1;
            chunk = IndexMap::new();
            chunk_pos = 0;
            chunk_len = 0;
        }
    }

    if !chunk.is_empty() {
        let chunk_handle = vm.arena.alloc(Val::Array(
            crate::core::value::ArrayData::from(chunk).into(),
        ));
        out.insert(ArrayKey::Int(chunk_index), chunk_handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_column(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_column() expects 2 or 3 parameters".into());
    }

    let input = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_column() expects parameter 1 to be array".into()),
    };

    let column_key = vm.arena.get(args[1]).value.clone();
    let index_key = if args.len() == 3 {
        Some(vm.arena.get(args[2]).value.clone())
    } else {
        None
    };

    let mut out = IndexMap::new();
    let mut idx = 0i64;

    for (_, row_handle) in input.map.iter() {
        let row_val = vm.arena.get(*row_handle);
        let row_arr = match &row_val.value {
            Val::Array(arr) => arr,
            _ => continue,
        };

        let col_key = array_key_from_val(&column_key);
        let value_handle = match col_key {
            Some(key) => row_arr.map.get(&key).copied(),
            None => None,
        };

        let value_handle = match value_handle {
            Some(handle) => handle,
            None => continue,
        };

        let out_key = if let Some(index_val) = &index_key {
            if let Some(key) = array_key_from_val(index_val) {
                if let Some(index_handle) = row_arr.map.get(&key) {
                    array_key_from_val(&vm.arena.get(*index_handle).value)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(key) = out_key {
            out.insert(key, value_handle);
        } else {
            out.insert(ArrayKey::Int(idx), value_handle);
            idx += 1;
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_combine(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_combine() expects exactly 2 parameters".into());
    }

    let keys = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_combine() expects parameter 1 to be array".into()),
    };
    let values = match &vm.arena.get(args[1]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_combine() expects parameter 2 to be array".into()),
    };

    if keys.map.len() != values.map.len() {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    let mut out = IndexMap::new();
    let mut key_iter = keys.map.values();
    let mut val_iter = values.map.values();
    while let (Some(key_handle), Some(val_handle)) = (key_iter.next(), val_iter.next()) {
        let key_val = vm.arena.get(*key_handle).value.clone();
        let key = array_key_from_val(&key_val)
            .ok_or("array_combine(): Argument #1 must contain only valid keys".to_string())?;
        out.insert(key, *val_handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_count_values(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_count_values() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_count_values() expects parameter 1 to be array".into()),
    };

    let mut counts: IndexMap<ArrayKey, i64> = IndexMap::new();
    for (_, value_handle) in arr.map.iter() {
        let val = &vm.arena.get(*value_handle).value;
        let key = match val {
            Val::Int(i) => ArrayKey::Int(*i),
            Val::String(s) => ArrayKey::Str(s.clone()),
            _ => continue,
        };
        let entry = counts.entry(key).or_insert(0);
        *entry += 1;
    }

    let mut out = IndexMap::new();
    for (key, count) in counts {
        out.insert(key, vm.arena.alloc(Val::Int(count)));
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_diff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_diff() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_diff() expects parameter 1 to be array".into()),
    };

    let mut other_values: Vec<Vec<u8>> = Vec::new();
    for (i, handle) in args[1..].iter().enumerate() {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => return Err(format!("array_diff(): Argument #{} is not an array", i + 2)),
        };
        for (_, val_handle) in arr.map.iter() {
            other_values.push(vm.arena.get(*val_handle).value.to_php_string_bytes());
        }
    }

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        let val_bytes = vm.arena.get(*value_handle).value.to_php_string_bytes();
        if !other_values.iter().any(|v| v == &val_bytes) {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_diff_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_diff_assoc() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_diff_assoc() expects parameter 1 to be array".into()),
    };

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        let val_bytes = vm.arena.get(*value_handle).value.to_php_string_bytes();
        let mut found = false;
        for handle in &args[1..] {
            let arr = match &vm.arena.get(*handle).value {
                Val::Array(arr) => arr,
                _ => continue,
            };
            if let Some(other_handle) = arr.map.get(key) {
                let other_bytes = vm.arena.get(*other_handle).value.to_php_string_bytes();
                if other_bytes == val_bytes {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_diff_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_diff_key() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_diff_key() expects parameter 1 to be array".into()),
    };

    let mut other_keys = Vec::new();
    for handle in &args[1..] {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => continue,
        };
        for key in arr.map.keys() {
            other_keys.push(key.clone());
        }
    }

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        if !other_keys.iter().any(|k| k == key) {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_diff_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_diff_uassoc() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_diff_uassoc() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_arrays: Vec<Vec<(ArrayKey, Handle)>> = args[1..args.len() - 1]
        .iter()
        .map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let val_bytes = vm.arena.get(value_handle).value.to_php_string_bytes();
        let mut found = false;
        for other in &other_arrays {
            for (other_key, other_val_handle) in other {
                let other_bytes = vm.arena.get(*other_val_handle).value.to_php_string_bytes();
                if other_bytes != val_bytes {
                    continue;
                }
                let key_handle = match &key {
                    ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                    ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
                };
                let other_key_handle = match other_key {
                    ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                    ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
                };
                let cmp = vm
                    .call_callable(callback, smallvec::smallvec![key_handle, other_key_handle])
                    .map_err(|e| format!("array_diff_uassoc(): {}", e))?;
                if vm.arena.get(cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_diff_ukey(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_diff_ukey() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_diff_ukey() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_keys: Vec<ArrayKey> = args[1..args.len() - 1]
        .iter()
        .flat_map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.keys().cloned().collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other_key in &other_keys {
            let key_handle = match &key {
                ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
            };
            let other_key_handle = match other_key {
                ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
            };
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![key_handle, other_key_handle])
                .map_err(|e| format!("array_diff_ukey(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() == 0 {
                found = true;
                break;
            }
        }
        if !found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_fill(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("array_fill() expects exactly 3 parameters".into());
    }

    let start_index = vm.check_builtin_param_int(args[0], 1, "array_fill")?;
    let count = vm.check_builtin_param_int(args[1], 2, "array_fill")?;
    if count < 0 {
        return Err("array_fill(): Number of elements must be greater than or equal to 0".into());
    }

    let mut map = IndexMap::new();
    for i in 0..count {
        map.insert(ArrayKey::Int(start_index + i), args[2]);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())))
}

pub fn php_array_fill_keys(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_fill_keys() expects exactly 2 parameters".into());
    }

    let keys = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.values().copied().collect::<Vec<_>>(),
        _ => return Err("array_fill_keys() expects parameter 1 to be array".into()),
    };

    let mut map = IndexMap::new();
    for handle in keys {
        let key_val = vm.arena.get(handle).value.clone();
        let key = array_key_from_val(&key_val)
            .ok_or("array_fill_keys(): Argument #1 must contain only valid keys".to_string())?;
        map.insert(key, args[1]);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())))
}

pub fn php_array_filter(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 3 {
        return Err("array_filter() expects between 1 and 3 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_filter() expects parameter 1 to be array".into()),
    };

    let callback = if args.len() >= 2 { Some(args[1]) } else { None };
    let mode = if args.len() == 3 {
        vm.arena.get(args[2]).value.to_int()
    } else {
        0
    };

    let mut out = IndexMap::new();
    for (key, value_handle) in entries {
        let keep = if let Some(cb) = callback {
            let key_handle = match &key {
                ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
            };
            let args = match mode {
                1 => smallvec::smallvec![key_handle],
                2 => smallvec::smallvec![value_handle, key_handle],
                _ => smallvec::smallvec![value_handle],
            };
            let result = vm
                .call_callable(cb, args)
                .map_err(|e| format!("array_filter(): {}", e))?;
            vm.arena.get(result).value.to_bool()
        } else {
            vm.arena.get(value_handle).value.to_bool()
        };

        if keep {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_find(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_find() expects exactly 2 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_find() expects parameter 1 to be array".into()),
    };

    let callback = args[1];
    for (key, value_handle) in entries {
        let key_handle = match &key {
            ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
            ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
        };
        let result = vm
            .call_callable(callback, smallvec::smallvec![value_handle, key_handle])
            .map_err(|e| format!("array_find(): {}", e))?;
        if vm.arena.get(result).value.to_bool() {
            return Ok(value_handle);
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_find_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_find_key() expects exactly 2 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_find_key() expects parameter 1 to be array".into()),
    };

    let callback = args[1];
    for (key, value_handle) in entries {
        let key_handle = match &key {
            ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
            ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
        };
        let result = vm
            .call_callable(callback, smallvec::smallvec![value_handle, key_handle])
            .map_err(|e| format!("array_find_key(): {}", e))?;
        if vm.arena.get(result).value.to_bool() {
            return Ok(key_handle);
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_first(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_first() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_first() expects parameter 1 to be array".into()),
    };

    if let Some((_, val_handle)) = arr.map.get_index(0) {
        return Ok(*val_handle);
    }
    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_flip(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_flip() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_flip() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Val)> = Vec::new();
    for (key, value_handle) in arr.map.iter() {
        let val = vm.arena.get(*value_handle).value.clone();
        let new_key = match val {
            Val::Int(i) => ArrayKey::Int(i),
            Val::String(s) => ArrayKey::Str(s),
            _ => {
                return Err("array_flip(): Can only flip string and integer values".into());
            }
        };
        let new_val = match key {
            ArrayKey::Int(i) => Val::Int(*i),
            ArrayKey::Str(s) => Val::String(s.clone()),
        };
        entries.push((new_key, new_val));
    }

    let mut out = IndexMap::new();
    for (key, val) in entries {
        out.insert(key, vm.arena.alloc(val));
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_intersect(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_intersect() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_intersect() expects parameter 1 to be array".into()),
    };

    let mut other_values: Vec<Vec<u8>> = Vec::new();
    for handle in &args[1..] {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => continue,
        };
        for (_, val_handle) in arr.map.iter() {
            other_values.push(vm.arena.get(*val_handle).value.to_php_string_bytes());
        }
    }

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        let val_bytes = vm.arena.get(*value_handle).value.to_php_string_bytes();
        if other_values.iter().any(|v| v == &val_bytes) {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_intersect_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_intersect_assoc() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_intersect_assoc() expects parameter 1 to be array".into()),
    };

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        let val_bytes = vm.arena.get(*value_handle).value.to_php_string_bytes();
        let mut found = false;
        for handle in &args[1..] {
            let arr = match &vm.arena.get(*handle).value {
                Val::Array(arr) => arr,
                _ => continue,
            };
            if let Some(other_handle) = arr.map.get(key) {
                let other_bytes = vm.arena.get(*other_handle).value.to_php_string_bytes();
                if other_bytes == val_bytes {
                    found = true;
                    break;
                }
            }
        }
        if found {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_intersect_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_intersect_key() expects at least 2 parameters".into());
    }

    let base = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_intersect_key() expects parameter 1 to be array".into()),
    };

    let mut other_keys = Vec::new();
    for handle in &args[1..] {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => continue,
        };
        for key in arr.map.keys() {
            other_keys.push(key.clone());
        }
    }

    let mut out = IndexMap::new();
    for (key, value_handle) in base.map.iter() {
        if other_keys.iter().any(|k| k == key) {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_intersect_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_intersect_uassoc() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_intersect_uassoc() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_arrays: Vec<Vec<(ArrayKey, Handle)>> = args[1..args.len() - 1]
        .iter()
        .map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let val_bytes = vm.arena.get(value_handle).value.to_php_string_bytes();
        let mut found = false;
        for other in &other_arrays {
            for (other_key, other_val_handle) in other {
                let other_bytes = vm.arena.get(*other_val_handle).value.to_php_string_bytes();
                if other_bytes != val_bytes {
                    continue;
                }
                let key_handle = match &key {
                    ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                    ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
                };
                let other_key_handle = match other_key {
                    ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                    ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
                };
                let cmp = vm
                    .call_callable(callback, smallvec::smallvec![key_handle, other_key_handle])
                    .map_err(|e| format!("array_intersect_uassoc(): {}", e))?;
                if vm.arena.get(cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_intersect_ukey(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_intersect_ukey() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_intersect_ukey() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_keys: Vec<ArrayKey> = args[1..args.len() - 1]
        .iter()
        .flat_map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.keys().cloned().collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other_key in &other_keys {
            let key_handle = match &key {
                ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
            };
            let other_key_handle = match other_key {
                ArrayKey::Int(i) => vm.arena.alloc(Val::Int(*i)),
                ArrayKey::Str(s) => vm.arena.alloc(Val::String(s.clone())),
            };
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![key_handle, other_key_handle])
                .map_err(|e| format!("array_intersect_ukey(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() == 0 {
                found = true;
                break;
            }
        }
        if found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_is_list(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_is_list() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_is_list() expects parameter 1 to be array".into()),
    };

    let mut index = 0i64;
    for key in arr.map.keys() {
        match key {
            ArrayKey::Int(i) if *i == index => {
                index += 1;
            }
            _ => {
                return Ok(vm.arena.alloc(Val::Bool(false)));
            }
        }
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_array_key_first(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_key_first() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_key_first() expects parameter 1 to be array".into()),
    };

    if let Some((key, _)) = arr.map.get_index(0) {
        let handle = vm.arena.alloc(array_key_to_val(key));
        return Ok(handle);
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_key_last(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_key_last() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_key_last() expects parameter 1 to be array".into()),
    };

    if let Some((key, _)) = arr.map.get_index(arr.map.len().saturating_sub(1)) {
        let handle = vm.arena.alloc(array_key_to_val(key));
        return Ok(handle);
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_last(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_last() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_last() expects parameter 1 to be array".into()),
    };

    if let Some((_, value_handle)) = arr.map.get_index(arr.map.len().saturating_sub(1)) {
        return Ok(*value_handle);
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_array_map(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_map() expects at least 2 parameters".into());
    }

    let callback = args[0];
    let callback_is_null = matches!(vm.arena.get(callback).value, Val::Null);

    let mut arrays: Vec<Vec<Handle>> = Vec::new();
    let mut first_entries: Vec<(ArrayKey, Handle)> = Vec::new();

    for (i, handle) in args[1..].iter().enumerate() {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => return Err(format!("array_map(): Argument #{} is not an array", i + 2)),
        };
        let entries: Vec<(ArrayKey, Handle)> =
            arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
        if i == 0 {
            first_entries = entries.clone();
        }
        arrays.push(entries.into_iter().map(|(_, v)| v).collect());
    }

    let mut out = IndexMap::new();
    for (idx, (key, _)) in first_entries.iter().enumerate() {
        let mut params = ArgList::new();
        for values in &arrays {
            if let Some(handle) = values.get(idx) {
                params.push(*handle);
            } else {
                params.push(vm.arena.alloc(Val::Null));
            }
        }

        let mapped = if callback_is_null {
            let mut row = IndexMap::new();
            for (i, handle) in params.iter().enumerate() {
                row.insert(ArrayKey::Int(i as i64), *handle);
            }
            vm.arena
                .alloc(Val::Array(crate::core::value::ArrayData::from(row).into()))
        } else {
            vm.call_callable(callback, params)
                .map_err(|e| format!("array_map(): {}", e))?
        };

        out.insert(key.clone(), mapped);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_merge_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("array_merge_recursive() expects at least 1 parameter".into());
    }

    let mut out = IndexMap::new();
    let mut next_int_key = 0i64;

    for (i, handle) in args.iter().enumerate() {
        let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => {
                return Err(format!(
                    "array_merge_recursive(): Argument #{} is not an array",
                    i + 1
                ));
            }
        };

        for (key, value_handle) in entries {
            match key {
                ArrayKey::Int(_) => {
                    out.insert(ArrayKey::Int(next_int_key), value_handle);
                    next_int_key += 1;
                }
                ArrayKey::Str(s) => {
                    let key_clone = ArrayKey::Str(s.clone());
                    if let Some(existing_handle) = out.get(&key_clone).copied() {
                        let merged = merge_recursive_values(vm, existing_handle, value_handle)?;
                        out.insert(key_clone, merged);
                    } else {
                        out.insert(key_clone, value_handle);
                    }
                }
            }
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_multisort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("array_multisort() expects at least 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_slot = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_slot.value {
        let mut arr_data = (**arr_rc).clone();

        let mut values: Vec<Handle> = arr_data.map.values().copied().collect();
        values.sort_by(|a, b| {
            let va = vm.arena.get(*a).value.to_float();
            let vb = vm.arena.get(*b).value.to_float();
            va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
        });

        arr_data.map.clear();
        for (idx, handle) in values.into_iter().enumerate() {
            arr_data.map.insert(ArrayKey::Int(idx as i64), handle);
        }
        arr_data.next_free = arr_data.map.len() as i64;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(vm.arena.alloc(Val::Bool(true)))
    } else {
        Err("array_multisort() expects parameter 1 to be array".into())
    }
}

pub fn php_array_pad(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 3 {
        return Err("array_pad() expects exactly 3 parameters".into());
    }

    let pad_size = vm.check_builtin_param_int(args[1], 2, "array_pad")?;
    let pad_value = args[2];
    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_pad() expects parameter 1 to be array".into()),
    };

    let mut values: Vec<Handle> = arr.map.values().copied().collect();
    let target_len = pad_size.unsigned_abs() as usize;
    if target_len <= values.len() {
        return Ok(vm.arena.alloc(Val::Array(
            crate::core::value::ArrayData::from(list_to_map(&values)).into(),
        )));
    }

    let pad_count = target_len - values.len();
    if pad_size >= 0 {
        values.extend(std::iter::repeat(pad_value).take(pad_count));
    } else {
        let mut padded = Vec::with_capacity(target_len);
        padded.extend(std::iter::repeat(pad_value).take(pad_count));
        padded.extend(values);
        values = padded;
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(list_to_map(&values)).into(),
    )))
}

pub fn php_array_pop(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_pop() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut arr_data = (**arr_rc).clone();
        let removed = if let Some((_key, value_handle)) = arr_data.map.pop() {
            value_handle
        } else {
            return Ok(vm.arena.alloc(Val::Null));
        };

        arr_data.next_free = next_int_key(&arr_data.map);
        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));
        Ok(removed)
    } else {
        Err("array_pop() expects parameter 1 to be array".into())
    }
}

pub fn php_array_product(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_product() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_product() expects parameter 1 to be array".into()),
    };

    let mut product = 1.0;
    for value_handle in arr.map.values() {
        product *= vm.arena.get(*value_handle).value.to_float();
    }

    if product.fract() == 0.0 {
        Ok(vm.arena.alloc(Val::Int(product as i64)))
    } else {
        Ok(vm.arena.alloc(Val::Float(product)))
    }
}

pub fn php_array_reduce(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_reduce() expects 2 or 3 parameters".into());
    }

    let values: Vec<Handle> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.values().copied().collect(),
        _ => return Err("array_reduce() expects parameter 1 to be array".into()),
    };

    let callback = args[1];
    let mut index = 0usize;
    let mut carry = if args.len() == 3 {
        args[2]
    } else if values.is_empty() {
        return Ok(vm.arena.alloc(Val::Null));
    } else {
        let first = values[0];
        index = 1;
        first
    };

    for value_handle in values.iter().skip(index) {
        carry = vm
            .call_callable(callback, smallvec::smallvec![carry, *value_handle])
            .map_err(|e| format!("array_reduce(): {}", e))?;
    }

    Ok(carry)
}

pub fn php_array_replace(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("array_replace() expects at least 1 parameter".into());
    }

    let mut out = IndexMap::new();
    for (i, handle) in args.iter().enumerate() {
        let arr = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr,
            _ => {
                return Err(format!(
                    "array_replace(): Argument #{} is not an array",
                    i + 1
                ));
            }
        };

        for (key, value_handle) in arr.map.iter() {
            out.insert(key.clone(), *value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_replace_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("array_replace_recursive() expects at least 1 parameter".into());
    }

    let mut out = IndexMap::new();
    for (i, handle) in args.iter().enumerate() {
        let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => {
                return Err(format!(
                    "array_replace_recursive(): Argument #{} is not an array",
                    i + 1
                ));
            }
        };

        for (key, value_handle) in entries {
            if let Some(existing_handle) = out.get(&key).copied() {
                if let (Val::Array(_), Val::Array(_)) = (
                    &vm.arena.get(existing_handle).value,
                    &vm.arena.get(value_handle).value,
                ) {
                    let merged = replace_recursive_values(vm, existing_handle, value_handle)?;
                    out.insert(key, merged);
                    continue;
                }
            }
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_reverse(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 1 || args.len() > 2 {
        return Err("array_reverse() expects 1 or 2 parameters".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_reverse() expects parameter 1 to be array".into()),
    };

    let preserve_keys = if args.len() == 2 {
        vm.arena.get(args[1]).value.to_bool()
    } else {
        false
    };

    let mut out = IndexMap::new();
    if preserve_keys {
        for (key, value_handle) in arr.map.iter().rev() {
            out.insert(key.clone(), *value_handle);
        }
    } else {
        let mut idx = 0i64;
        for (_, value_handle) in arr.map.iter().rev() {
            out.insert(ArrayKey::Int(idx), *value_handle);
            idx += 1;
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_search(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_search() expects 2 or 3 parameters".into());
    }

    let needle = vm.arena.get(args[0]).value.clone();
    let haystack = match &vm.arena.get(args[1]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_search() expects parameter 2 to be array".into()),
    };

    let strict = if args.len() == 3 {
        vm.arena.get(args[2]).value.to_bool()
    } else {
        false
    };

    for (key, value_handle) in haystack.map.iter() {
        let candidate = vm.arena.get(*value_handle).value.clone();
        if values_equal(&needle, &candidate, strict) {
            return Ok(vm.arena.alloc(array_key_to_val(key)));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_array_shift(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_shift() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut arr_data = (**arr_rc).clone();
        let mut iter = arr_data.map.into_iter();
        let removed = if let Some((_key, value_handle)) = iter.next() {
            value_handle
        } else {
            return Ok(vm.arena.alloc(Val::Null));
        };

        let mut new_map = IndexMap::new();
        let mut next_int = 0i64;
        for (key, value_handle) in iter {
            match key {
                ArrayKey::Int(_) => {
                    new_map.insert(ArrayKey::Int(next_int), value_handle);
                    next_int += 1;
                }
                ArrayKey::Str(s) => {
                    new_map.insert(ArrayKey::Str(s), value_handle);
                }
            }
        }

        arr_data.map = new_map;
        arr_data.next_free = next_int;
        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(removed)
    } else {
        Err("array_shift() expects parameter 1 to be array".into())
    }
}

pub fn php_array_slice(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 4 {
        return Err("array_slice() expects between 2 and 4 parameters".into());
    }

    let mut offset = vm.check_builtin_param_int(args[1], 2, "array_slice")?;
    let length = if args.len() >= 3 {
        Some(vm.check_builtin_param_int(args[2], 3, "array_slice")?)
    } else {
        None
    };
    let preserve_keys = if args.len() == 4 {
        vm.arena.get(args[3]).value.to_bool()
    } else {
        false
    };
    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_slice() expects parameter 1 to be array".into()),
    };

    let len = arr.map.len() as i64;
    if offset < 0 {
        offset = (len + offset).max(0);
    }

    let mut end = len;
    if let Some(length) = length {
        if length >= 0 {
            end = (offset + length).min(len);
        } else {
            end = (len + length).max(offset);
        }
    }

    let mut out = IndexMap::new();
    let mut idx = 0i64;
    for (i, (key, value_handle)) in arr.map.iter().enumerate() {
        let i = i as i64;
        if i < offset || i >= end {
            continue;
        }
        if preserve_keys {
            out.insert(key.clone(), *value_handle);
        } else {
            out.insert(ArrayKey::Int(idx), *value_handle);
            idx += 1;
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_splice(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 4 {
        return Err("array_splice() expects between 2 and 4 parameters".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);
    let mut arr_data = match &arr_val.value {
        Val::Array(arr) => (**arr).clone(),
        _ => return Err("array_splice() expects parameter 1 to be array".into()),
    };

    let len = arr_data.map.len() as i64;
    let mut offset = vm.check_builtin_param_int(args[1], 2, "array_splice")?;
    if offset < 0 {
        offset = (len + offset).max(0);
    }

    let length = if args.len() >= 3 {
        vm.check_builtin_param_int(args[2], 3, "array_splice")?
    } else {
        len - offset
    };
    let length = length.max(0);

    let mut values: Vec<Handle> = arr_data.map.values().copied().collect();
    let start = offset as usize;
    let end = (offset + length).min(len) as usize;

    let removed_values = values[start..end].to_vec();
    values.splice(start..end, replacement_values(vm, args.get(3).copied()));

    arr_data.map = list_to_map(&values);
    arr_data.next_free = arr_data.map.len() as i64;
    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(list_to_map(&removed_values)).into(),
    )))
}

pub fn php_array_sum(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_sum() expects exactly 1 parameter".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_sum() expects parameter 1 to be array".into()),
    };

    let mut sum = 0.0;
    for value_handle in arr.map.values() {
        sum += vm.arena.get(*value_handle).value.to_float();
    }

    if sum.fract() == 0.0 {
        Ok(vm.arena.alloc(Val::Int(sum as i64)))
    } else {
        Ok(vm.arena.alloc(Val::Float(sum)))
    }
}

pub fn php_array_rand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("array_rand() expects 1 or 2 parameters".into());
    }

    let entries: Vec<ArrayKey> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.keys().cloned().collect(),
        _ => return Err("array_rand() expects parameter 1 to be array".into()),
    };

    let count = entries.len();
    if count == 0 {
        return Err("array_rand(): Array is empty".into());
    }

    let num = if args.len() == 2 {
        vm.check_builtin_param_int(args[1], 2, "array_rand")?
    } else {
        1
    };

    if num < 1 || num as usize > count {
        return Err(
            "array_rand(): Number of elements must be between 1 and the number of elements".into(),
        );
    }

    let core = vm
        .context
        .get_or_init_extension_data(CoreExtensionData::default);

    let mut keys = entries;
    if num as usize == 1 {
        let idx = (core.rng_next_u32() as usize) % count;
        let handle = vm.arena.alloc(array_key_to_val(&keys[idx]));
        return Ok(handle);
    }

    // Fisher-Yates shuffle for deterministic unique selection
    for i in (1..keys.len()).rev() {
        let j = (core.rng_next_u32() as usize) % (i + 1);
        keys.swap(i, j);
    }

    let mut out = IndexMap::new();
    for (idx, key) in keys.into_iter().take(num as usize).enumerate() {
        let handle = vm.arena.alloc(array_key_to_val(&key));
        out.insert(ArrayKey::Int(idx as i64), handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_udiff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_udiff() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_udiff() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_values: Vec<Handle> = args[1..args.len() - 1]
        .iter()
        .flat_map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.values().copied().collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other_handle in &other_values {
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![value_handle, *other_handle])
                .map_err(|e| format!("array_udiff(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() == 0 {
                found = true;
                break;
            }
        }
        if !found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_udiff_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_udiff_assoc() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_udiff_assoc() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for handle in &args[1..args.len() - 1] {
            let arr = match &vm.arena.get(*handle).value {
                Val::Array(arr) => arr,
                _ => continue,
            };
            if let Some(other_handle) = arr.map.get(&key) {
                let cmp = vm
                    .call_callable(callback, smallvec::smallvec![value_handle, *other_handle])
                    .map_err(|e| format!("array_udiff_assoc(): {}", e))?;
                if vm.arena.get(cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_udiff_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 4 {
        return Err("array_udiff_uassoc() expects at least 4 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_udiff_uassoc() expects parameter 1 to be array".into()),
    };

    let value_cb = args[args.len() - 2];
    let key_cb = args[args.len() - 1];
    let other_arrays: Vec<Vec<(ArrayKey, Handle)>> = args[1..args.len() - 2]
        .iter()
        .map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other in &other_arrays {
            for (other_key, other_handle) in other {
                let val_cmp = vm
                    .call_callable(value_cb, smallvec::smallvec![value_handle, *other_handle])
                    .map_err(|e| format!("array_udiff_uassoc(): {}", e))?;
                if vm.arena.get(val_cmp).value.to_int() != 0 {
                    continue;
                }

                let key_handle = vm.arena.alloc(array_key_to_val(&key));
                let other_key_handle = vm.arena.alloc(array_key_to_val(other_key));
                let key_cmp = vm
                    .call_callable(key_cb, smallvec::smallvec![key_handle, other_key_handle])
                    .map_err(|e| format!("array_udiff_uassoc(): {}", e))?;
                if vm.arena.get(key_cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_uintersect(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_uintersect() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_uintersect() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];
    let other_values: Vec<Handle> = args[1..args.len() - 1]
        .iter()
        .flat_map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.values().copied().collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other_handle in &other_values {
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![value_handle, *other_handle])
                .map_err(|e| format!("array_uintersect(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() == 0 {
                found = true;
                break;
            }
        }
        if found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_uintersect_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 3 {
        return Err("array_uintersect_assoc() expects at least 3 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_uintersect_assoc() expects parameter 1 to be array".into()),
    };

    let callback = args[args.len() - 1];

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for handle in &args[1..args.len() - 1] {
            let arr = match &vm.arena.get(*handle).value {
                Val::Array(arr) => arr,
                _ => continue,
            };
            if let Some(other_handle) = arr.map.get(&key) {
                let cmp = vm
                    .call_callable(callback, smallvec::smallvec![value_handle, *other_handle])
                    .map_err(|e| format!("array_uintersect_assoc(): {}", e))?;
                if vm.arena.get(cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
        }
        if found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_uintersect_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 4 {
        return Err("array_uintersect_uassoc() expects at least 4 parameters".into());
    }

    let base_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_uintersect_uassoc() expects parameter 1 to be array".into()),
    };

    let value_cb = args[args.len() - 2];
    let key_cb = args[args.len() - 1];
    let other_arrays: Vec<Vec<(ArrayKey, Handle)>> = args[1..args.len() - 2]
        .iter()
        .map(|handle| match &vm.arena.get(*handle).value {
            Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            _ => Vec::new(),
        })
        .collect();

    let mut out = IndexMap::new();
    for (key, value_handle) in base_entries {
        let mut found = false;
        for other in &other_arrays {
            for (other_key, other_handle) in other {
                let val_cmp = vm
                    .call_callable(value_cb, smallvec::smallvec![value_handle, *other_handle])
                    .map_err(|e| format!("array_uintersect_uassoc(): {}", e))?;
                if vm.arena.get(val_cmp).value.to_int() != 0 {
                    continue;
                }

                let key_handle = vm.arena.alloc(array_key_to_val(&key));
                let other_key_handle = vm.arena.alloc(array_key_to_val(other_key));
                let key_cmp = vm
                    .call_callable(key_cb, smallvec::smallvec![key_handle, other_key_handle])
                    .map_err(|e| format!("array_uintersect_uassoc(): {}", e))?;
                if vm.arena.get(key_cmp).value.to_int() == 0 {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if found {
            out.insert(key, value_handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_unique(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("array_unique() expects 1 or 2 parameters".into());
    }

    let arr = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr,
        _ => return Err("array_unique() expects parameter 1 to be array".into()),
    };

    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut out = IndexMap::new();
    for (key, value_handle) in arr.map.iter() {
        let val_bytes = vm.arena.get(*value_handle).value.to_php_string_bytes();
        if seen.iter().any(|v| v == &val_bytes) {
            continue;
        }
        seen.push(val_bytes);
        out.insert(key.clone(), *value_handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_array_walk(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_walk() expects 2 or 3 parameters".into());
    }

    let arr_handle = args[0];
    let callback = args[1];
    let user_data = if args.len() == 3 { Some(args[2]) } else { None };

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(arr_handle).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_walk() expects parameter 1 to be array".into()),
    };

    for (key, value_handle) in entries {
        let key_handle = vm.arena.alloc(array_key_to_val(&key));
        let mut params = ArgList::new();
        params.push(value_handle);
        params.push(key_handle);
        if let Some(extra) = user_data {
            params.push(extra);
        }
        vm.call_callable(callback, params)
            .map_err(|e| format!("array_walk(): {}", e))?;
    }

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_array_walk_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("array_walk_recursive() expects 2 or 3 parameters".into());
    }

    let callback = args[1];
    let user_data = if args.len() == 3 { Some(args[2]) } else { None };

    let arr_handle = args[0];
    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(arr_handle).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("array_walk_recursive() expects parameter 1 to be array".into()),
    };

    fn walk_inner(
        vm: &mut VM,
        entries: Vec<(ArrayKey, Handle)>,
        callback: Handle,
        user_data: Option<Handle>,
    ) -> Result<(), String> {
        for (key, value_handle) in entries {
            let value = vm.arena.get(value_handle).value.clone();
            if let Val::Array(arr) = value {
                let child_entries = arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
                walk_inner(vm, child_entries, callback, user_data)?;
                continue;
            }

            let key_handle = vm.arena.alloc(array_key_to_val(&key));
            let mut params = ArgList::new();
            params.push(value_handle);
            params.push(key_handle);
            if let Some(extra) = user_data {
                params.push(extra);
            }
            vm.call_callable(callback, params)
                .map_err(|e| format!("array_walk_recursive(): {}", e))?;
        }
        Ok(())
    }

    walk_inner(vm, entries, callback, user_data)?;
    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_arsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("arsort() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("arsort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Handle)> =
        arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();

    entries.sort_by(|(_, a), (_, b)| {
        let av = vm.arena.get(*a).value.to_float();
        let bv = vm.arena.get(*b).value.to_float();
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_asort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("asort() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("asort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Handle)> =
        arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();

    entries.sort_by(|(_, a), (_, b)| {
        let av = vm.arena.get(*a).value.to_float();
        let bv = vm.arena.get(*b).value.to_float();
        av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
    });

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_compact(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("compact() expects at least 1 parameter".into());
    }

    let mut names: Vec<Vec<u8>> = Vec::new();
    for handle in args {
        match &vm.arena.get(*handle).value {
            Val::String(s) => names.push((**s).clone()),
            Val::Array(arr) => {
                for (_, value_handle) in arr.map.iter() {
                    let val = vm.arena.get(*value_handle).value.to_php_string_bytes();
                    names.push(val);
                }
            }
            _ => continue,
        }
    }

    let mut out = IndexMap::new();
    for name in names {
        let sym = vm.context.interner.intern(&name);
        if let Some(handle) = vm
            .frames
            .last()
            .and_then(|frame| frame.locals.get(&sym).copied())
        {
            out.insert(ArrayKey::Str(std::rc::Rc::new(name)), handle);
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_each(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("each() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let key_value = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => arr_rc.map.clone(),
        _ => return Err("each() expects parameter 1 to be array".into()),
    };

    let keys_values: Vec<(ArrayKey, Handle)> =
        key_value.iter().map(|(k, v)| (k.clone(), *v)).collect();

    let len = keys_values.len();
    if len == 0 {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    let ext_data = vm
        .context
        .get_or_init_extension_data(CoreExtensionData::default);
    let pos = ext_data.array_pointers.entry(arr_handle).or_insert(0);
    if *pos >= len {
        return Ok(vm.arena.alloc(Val::Bool(false)));
    }

    let (key, value_handle) = keys_values.get(*pos).unwrap();
    *pos += 1;

    let key_handle = vm.arena.alloc(array_key_to_val(key));

    let mut out = IndexMap::new();
    out.insert(ArrayKey::Int(0), key_handle);
    out.insert(ArrayKey::Int(1), *value_handle);
    out.insert(ArrayKey::Str(std::rc::Rc::new(b"key".to_vec())), key_handle);
    out.insert(
        ArrayKey::Str(std::rc::Rc::new(b"value".to_vec())),
        *value_handle,
    );

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_extract(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() || args.len() > 2 {
        return Err("extract() expects 1 or 2 parameters".into());
    }

    let entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(args[0]).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Err("extract() expects parameter 1 to be array".into()),
    };

    let mut count = 0;
    for (key, value_handle) in entries {
        let name = match key {
            ArrayKey::Int(i) => i.to_string().into_bytes(),
            ArrayKey::Str(s) => (*s).clone(),
        };
        let sym = vm.context.interner.intern(&name);
        vm.store_variable(sym, value_handle)
            .map_err(|e| format!("extract(): {}", e))?;
        count += 1;
    }

    Ok(vm.arena.alloc(Val::Int(count)))
}

pub fn php_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("key() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len == 0 {
            return Ok(vm.arena.alloc(Val::Null));
        }
        let ext_data = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);
        let pos = ext_data.array_pointers.entry(arr_handle).or_insert(0);
        if *pos >= len {
            return Ok(vm.arena.alloc(Val::Null));
        }
        if let Some((key, _)) = arr_rc.map.get_index(*pos) {
            return Ok(vm.arena.alloc(array_key_to_val(key)));
        }
    }

    Ok(vm.arena.alloc(Val::Null))
}

pub fn php_key_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_key_exists(vm, args)
}

pub fn php_krsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("krsort() expects at least 1 parameter".into());
    }

    let arr_handle = args[0];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("krsort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<_> = arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    entries.sort_by(|(a, _), (b, _)| match (a, b) {
        (ArrayKey::Int(i1), ArrayKey::Int(i2)) => i2.cmp(i1),
        (ArrayKey::Str(s1), ArrayKey::Str(s2)) => s2.cmp(s1),
        (ArrayKey::Int(_), ArrayKey::Str(_)) => std::cmp::Ordering::Greater,
        (ArrayKey::Str(_), ArrayKey::Int(_)) => std::cmp::Ordering::Less,
    });

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_natcasesort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_natsort_internal(vm, args, false)
}

pub fn php_natsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_natsort_internal(vm, args, true)
}

fn php_natsort_internal(
    vm: &mut VM,
    args: &[Handle],
    case_sensitive: bool,
) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("natsort() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("natsort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Handle)> =
        arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();

    entries.sort_by(|(_, a), (_, b)| {
        let a_str = vm.arena.get(*a).value.to_php_string_bytes();
        let b_str = vm.arena.get(*b).value.to_php_string_bytes();
        natural_cmp(&a_str, &b_str, case_sensitive)
    });

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_pos(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_current(vm, args)
}

pub fn php_prev(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("prev() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len == 0 {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        let ext_data = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);
        let pos = ext_data.array_pointers.entry(arr_handle).or_insert(0);
        if *pos == 0 {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        *pos -= 1;
        if let Some((_, val_handle)) = arr_rc.map.get_index(*pos) {
            return Ok(*val_handle);
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_range(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("range() expects 2 or 3 parameters".into());
    }

    let start = vm.arena.get(args[0]).value.to_int();
    let end = vm.arena.get(args[1]).value.to_int();
    let step = if args.len() == 3 {
        vm.arena.get(args[2]).value.to_int()
    } else {
        if start <= end { 1 } else { -1 }
    };

    if step == 0 {
        return Err("range(): step must not be 0".into());
    }

    let mut values = Vec::new();
    let mut current = start;
    if step > 0 {
        while current <= end {
            values.push(vm.arena.alloc(Val::Int(current)));
            current += step;
        }
    } else {
        while current >= end {
            values.push(vm.arena.alloc(Val::Int(current)));
            current += step;
        }
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(list_to_map(&values)).into(),
    )))
}

pub fn php_rsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("rsort() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut values: Vec<Handle> = arr_rc.map.values().copied().collect();
        values.sort_by(|a, b| {
            let av = vm.arena.get(*a).value.to_float();
            let bv = vm.arena.get(*b).value.to_float();
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut arr_data = (**arr_rc).clone();
        arr_data.map = list_to_map(&values);
        arr_data.next_free = arr_data.map.len() as i64;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(vm.arena.alloc(Val::Bool(true)))
    } else {
        Err("rsort() expects parameter 1 to be array".into())
    }
}

pub fn php_shuffle(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("shuffle() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut values: Vec<Handle> = arr_rc.map.values().copied().collect();
        let core = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);

        for i in (1..values.len()).rev() {
            let j = (core.rng_next_u32() as usize) % (i + 1);
            values.swap(i, j);
        }

        let mut arr_data = (**arr_rc).clone();
        arr_data.map = list_to_map(&values);
        arr_data.next_free = arr_data.map.len() as i64;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(vm.arena.alloc(Val::Bool(true)))
    } else {
        Err("shuffle() expects parameter 1 to be array".into())
    }
}

pub fn php_sizeof(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_count(vm, args)
}

pub fn php_sort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("sort() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut values: Vec<Handle> = arr_rc.map.values().copied().collect();
        values.sort_by(|a, b| {
            let av = vm.arena.get(*a).value.to_float();
            let bv = vm.arena.get(*b).value.to_float();
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut arr_data = (**arr_rc).clone();
        arr_data.map = list_to_map(&values);
        arr_data.next_free = arr_data.map.len() as i64;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(vm.arena.alloc(Val::Bool(true)))
    } else {
        Err("sort() expects parameter 1 to be array".into())
    }
}

pub fn php_uasort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("uasort() expects at least 2 parameters".into());
    }

    let arr_handle = args[0];
    let callback = args[1];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("uasort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Handle)> =
        arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let len = entries.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![entries[i].1, entries[j].1])
                .map_err(|e| format!("uasort(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() > 0 {
                entries.swap(i, j);
            }
        }
    }

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_uksort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("uksort() expects at least 2 parameters".into());
    }

    let arr_handle = args[0];
    let callback = args[1];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("uksort() expects parameter 1 to be array".into()),
    };

    let mut entries: Vec<(ArrayKey, Handle)> =
        arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let len = entries.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let key_i = vm.arena.alloc(array_key_to_val(&entries[i].0));
            let key_j = vm.arena.alloc(array_key_to_val(&entries[j].0));
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![key_i, key_j])
                .map_err(|e| format!("uksort(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() > 0 {
                entries.swap(i, j);
            }
        }
    }

    let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
    arr_data.map = sorted_map;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

pub fn php_usort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("usort() expects at least 2 parameters".into());
    }

    let arr_handle = args[0];
    let callback = args[1];
    let mut arr_data = match &vm.arena.get(arr_handle).value {
        Val::Array(arr_rc) => (**arr_rc).clone(),
        _ => return Err("usort() expects parameter 1 to be array".into()),
    };

    let mut values: Vec<Handle> = arr_data.map.values().copied().collect();
    let len = values.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let cmp = vm
                .call_callable(callback, smallvec::smallvec![values[i], values[j]])
                .map_err(|e| format!("usort(): {}", e))?;
            if vm.arena.get(cmp).value.to_int() > 0 {
                values.swap(i, j);
            }
        }
    }

    arr_data.map = list_to_map(&values);
    arr_data.next_free = arr_data.map.len() as i64;

    let slot = vm.arena.get_mut(arr_handle);
    slot.value = Val::Array(std::rc::Rc::new(arr_data));

    Ok(vm.arena.alloc(Val::Bool(true)))
}

fn natural_cmp(a: &[u8], b: &[u8], case_sensitive: bool) -> std::cmp::Ordering {
    let mut ia = 0;
    let mut ib = 0;

    while ia < a.len() && ib < b.len() {
        let ca = a[ia];
        let cb = b[ib];

        let a_is_digit = ca.is_ascii_digit();
        let b_is_digit = cb.is_ascii_digit();

        if a_is_digit && b_is_digit {
            let mut enda = ia;
            let mut endb = ib;
            while enda < a.len() && a[enda].is_ascii_digit() {
                enda += 1;
            }
            while endb < b.len() && b[endb].is_ascii_digit() {
                endb += 1;
            }

            let num_a = std::str::from_utf8(&a[ia..enda]).unwrap_or("0");
            let num_b = std::str::from_utf8(&b[ib..endb]).unwrap_or("0");
            let int_a = num_a.parse::<u64>().unwrap_or(0);
            let int_b = num_b.parse::<u64>().unwrap_or(0);
            if int_a != int_b {
                return int_a.cmp(&int_b);
            }
            ia = enda;
            ib = endb;
            continue;
        }

        let mut cha = ca;
        let mut chb = cb;
        if !case_sensitive {
            cha = cha.to_ascii_lowercase();
            chb = chb.to_ascii_lowercase();
        }
        if cha != chb {
            return cha.cmp(&chb);
        }
        ia += 1;
        ib += 1;
    }

    a.len().cmp(&b.len())
}

pub fn php_array_merge(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    let mut new_array = IndexMap::new();
    let mut next_int_key = 0;

    for (i, arg_handle) in args.iter().enumerate() {
        let val = vm.arena.get(*arg_handle);
        match &val.value {
            Val::Array(arr) => {
                for (key, value_handle) in arr.map.iter() {
                    match key {
                        ArrayKey::Int(_) => {
                            new_array.insert(ArrayKey::Int(next_int_key), *value_handle);
                            next_int_key += 1;
                        }
                        ArrayKey::Str(s) => {
                            new_array.insert(ArrayKey::Str(s.clone()), *value_handle);
                        }
                    }
                }
            }
            _ => {
                return Err(format!(
                    "array_merge(): Argument #{} is not an array",
                    i + 1
                ));
            }
        }
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(new_array).into(),
    )))
}

pub fn php_array_keys(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 1 {
        return Err("array_keys() expects at least 1 parameter".into());
    }

    let keys: Vec<ArrayKey> = {
        let val = vm.arena.get(args[0]);
        let arr = match &val.value {
            Val::Array(arr) => arr,
            _ => return Err("array_keys() expects parameter 1 to be array".into()),
        };
        arr.map.keys().cloned().collect()
    };

    let mut keys_arr = IndexMap::new();
    let mut idx = 0;

    for key in keys {
        let key_val = match key {
            ArrayKey::Int(i) => Val::Int(i),
            ArrayKey::Str(s) => Val::String((*s).clone().into()),
        };
        let key_handle = vm.arena.alloc(key_val);
        keys_arr.insert(ArrayKey::Int(idx), key_handle);
        idx += 1;
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(keys_arr).into(),
    )))
}

pub fn php_array_values(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 1 {
        return Err("array_values() expects exactly 1 parameter".into());
    }

    let val = vm.arena.get(args[0]);
    let arr = match &val.value {
        Val::Array(arr) => arr,
        _ => return Err("array_values() expects parameter 1 to be array".into()),
    };

    let mut values_arr = IndexMap::new();
    let mut idx = 0;

    for (_, value_handle) in arr.map.iter() {
        values_arr.insert(ArrayKey::Int(idx), *value_handle);
        idx += 1;
    }

    Ok(vm.arena.alloc(Val::Array(
        crate::core::value::ArrayData::from(values_arr).into(),
    )))
}

pub fn php_in_array(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err("in_array() expects 2 or 3 parameters".into());
    }

    let needle = vm.arena.get(args[0]).value.clone();

    let haystack = match &vm.arena.get(args[1]).value {
        Val::Array(arr) => arr,
        _ => return Err("in_array(): Argument #2 ($haystack) must be of type array".into()),
    };

    let strict = if args.len() == 3 {
        vm.arena.get(args[2]).value.to_bool()
    } else {
        false
    };

    for (_, value_handle) in haystack.map.iter() {
        let candidate = vm.arena.get(*value_handle).value.clone();
        if values_equal(&needle, &candidate, strict) {
            return Ok(vm.arena.alloc(Val::Bool(true)));
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

fn values_equal(a: &Val, b: &Val, strict: bool) -> bool {
    if strict {
        return a == b;
    }

    match (a, b) {
        (Val::Bool(_), _) | (_, Val::Bool(_)) => a.to_bool() == b.to_bool(),
        (Val::Int(_), Val::Int(_)) => a == b,
        (Val::Float(_), Val::Float(_)) => a == b,
        (Val::Int(_), Val::Float(_)) | (Val::Float(_), Val::Int(_)) => a.to_float() == b.to_float(),
        (Val::String(_), Val::String(_)) => a == b,
        (Val::String(_), Val::Int(_))
        | (Val::Int(_), Val::String(_))
        | (Val::String(_), Val::Float(_))
        | (Val::Float(_), Val::String(_)) => a.to_float() == b.to_float(),
        _ => a == b,
    }
}

pub fn php_ksort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("ksort() expects at least 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_slot = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_slot.value {
        let mut arr_data = (**arr_rc).clone();

        // Sort keys: collect entries, sort, and rebuild
        let mut entries: Vec<_> = arr_data.map.iter().map(|(k, v)| (k.clone(), *v)).collect();
        entries.sort_by(|(a, _), (b, _)| match (a, b) {
            (ArrayKey::Int(i1), ArrayKey::Int(i2)) => i1.cmp(i2),
            (ArrayKey::Str(s1), ArrayKey::Str(s2)) => s1.cmp(s2),
            (ArrayKey::Int(_), ArrayKey::Str(_)) => std::cmp::Ordering::Less,
            (ArrayKey::Str(_), ArrayKey::Int(_)) => std::cmp::Ordering::Greater,
        });

        let sorted_map: IndexMap<_, _> = entries.into_iter().collect();
        arr_data.map = sorted_map;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        Ok(vm.arena.alloc(Val::Bool(true)))
    } else {
        Err("ksort() expects parameter 1 to be array".into())
    }
}

pub fn php_array_unshift(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("array_unshift() expects at least 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut arr_data = (**arr_rc).clone();
        let old_len = arr_data.map.len() as i64;

        // Rebuild array with new elements prepended
        let mut new_map = IndexMap::new();

        // Add new elements first (from args[1..])
        for (i, &arg) in args[1..].iter().enumerate() {
            new_map.insert(ArrayKey::Int(i as i64), arg);
        }

        // Then add existing elements with shifted indices
        let shift_by = (args.len() - 1) as i64;
        for (key, val_handle) in &arr_data.map {
            match key {
                ArrayKey::Int(idx) => {
                    new_map.insert(ArrayKey::Int(idx + shift_by), *val_handle);
                }
                ArrayKey::Str(s) => {
                    new_map.insert(ArrayKey::Str(s.clone()), *val_handle);
                }
            }
        }

        arr_data.map = new_map;
        arr_data.next_free += shift_by;

        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(std::rc::Rc::new(arr_data));

        let new_len = old_len + shift_by;
        Ok(vm.arena.alloc(Val::Int(new_len)))
    } else {
        Err("array_unshift() expects parameter 1 to be array".into())
    }
}

pub fn php_array_push(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() < 2 {
        return Err("array_push() expects at least 2 parameters".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let mut arr_data = (**arr_rc).clone();
        let mut next_index = arr_data
            .map
            .keys()
            .filter_map(|key| {
                if let ArrayKey::Int(idx) = key {
                    Some(*idx)
                } else {
                    None
                }
            })
            .max()
            .map(|last| last + 1)
            .unwrap_or(0);

        for &value in &args[1..] {
            arr_data.map.insert(ArrayKey::Int(next_index), value);
            next_index += 1;
        }

        let arr_rc_new = Rc::new(arr_data);
        let count = arr_rc_new.map.len() as i64;
        let slot = vm.arena.get_mut(arr_handle);
        slot.value = Val::Array(arr_rc_new);

        Ok(vm.arena.alloc(Val::Int(count)))
    } else {
        Err("array_push() expects parameter 1 to be array".into())
    }
}

pub fn php_current(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("current() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len == 0 {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        let ext_data = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);
        let pos = ext_data.array_pointers.entry(arr_handle).or_insert(0);
        if *pos >= len {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        if let Some((_, val_handle)) = arr_rc.map.get_index(*pos) {
            Ok(*val_handle)
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else if let Val::ConstArray(map) = &arr_val.value {
        if let Some((_, val)) = map.iter().next() {
            Ok(vm.arena.alloc(val.clone()))
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else {
        Ok(vm.arena.alloc(Val::Bool(false)))
    }
}

pub fn php_next(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("next() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len == 0 {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        let ext_data = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);
        let pos = ext_data.array_pointers.entry(arr_handle).or_insert(0);
        if *pos + 1 >= len {
            *pos = len;
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        *pos += 1;
        if let Some((_, val_handle)) = arr_rc.map.get_index(*pos) {
            return Ok(*val_handle);
        }
    }

    Ok(vm.arena.alloc(Val::Bool(false)))
}

pub fn php_reset(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("reset() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len == 0 {
            return Ok(vm.arena.alloc(Val::Bool(false)));
        }
        let ext_data = vm
            .context
            .get_or_init_extension_data(CoreExtensionData::default);
        ext_data.array_pointers.insert(arr_handle, 0);
        if let Some((_, val_handle)) = arr_rc.map.get_index(0) {
            Ok(*val_handle)
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else if let Val::ConstArray(map) = &arr_val.value {
        if let Some((_, val)) = map.iter().next() {
            Ok(vm.arena.alloc(val.clone()))
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else {
        Ok(vm.arena.alloc(Val::Bool(false)))
    }
}

pub fn php_end(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.is_empty() {
        return Err("end() expects exactly 1 parameter".into());
    }

    let arr_handle = args[0];
    let arr_val = vm.arena.get(arr_handle);

    if let Val::Array(arr_rc) = &arr_val.value {
        let len = arr_rc.map.len();
        if len > 0 {
            let ext_data = vm
                .context
                .get_or_init_extension_data(CoreExtensionData::default);
            ext_data.array_pointers.insert(arr_handle, len - 1);
            if let Some((_, val_handle)) = arr_rc.map.get_index(len - 1) {
                Ok(*val_handle)
            } else {
                Ok(vm.arena.alloc(Val::Bool(false)))
            }
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else if let Val::ConstArray(map) = &arr_val.value {
        if let Some((_, val)) = map.iter().last() {
            Ok(vm.arena.alloc(val.clone()))
        } else {
            Ok(vm.arena.alloc(Val::Bool(false)))
        }
    } else {
        Ok(vm.arena.alloc(Val::Bool(false)))
    }
}

pub fn php_array_key_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    if args.len() != 2 {
        return Err("array_key_exists() expects exactly 2 parameters".into());
    }

    let key_val = vm.arena.get(args[0]).value.clone();
    let arr_val = vm.arena.get(args[1]);

    if let Val::Array(arr_rc) = &arr_val.value {
        let key = match key_val {
            Val::Int(i) => ArrayKey::Int(i),
            Val::String(s) => ArrayKey::Str(s.into()),
            Val::Float(f) => ArrayKey::Int(f as i64),
            Val::Bool(b) => ArrayKey::Int(if b { 1 } else { 0 }),
            Val::Null => ArrayKey::Str(vec![].into()),
            _ => {
                return Err(
                    "array_key_exists(): Argument #1 ($key) must be a valid array key".into(),
                );
            }
        };

        let exists = arr_rc.map.contains_key(&key);
        Ok(vm.arena.alloc(Val::Bool(exists)))
    } else {
        Err("array_key_exists(): Argument #2 ($array) must be of type array".into())
    }
}

fn array_key_from_val(val: &Val) -> Option<ArrayKey> {
    match val {
        Val::Int(i) => Some(ArrayKey::Int(*i)),
        Val::String(s) => Some(ArrayKey::Str(s.clone())),
        Val::Float(f) => Some(ArrayKey::Int(*f as i64)),
        Val::Bool(b) => Some(ArrayKey::Int(if *b { 1 } else { 0 })),
        Val::Null => Some(ArrayKey::Str(Rc::new(Vec::new()))),
        _ => None,
    }
}

fn array_key_to_val(key: &ArrayKey) -> Val {
    match key {
        ArrayKey::Int(i) => Val::Int(*i),
        ArrayKey::Str(s) => Val::String(s.clone()),
    }
}

fn list_to_map(values: &[Handle]) -> IndexMap<ArrayKey, Handle> {
    let mut map = IndexMap::new();
    for (idx, handle) in values.iter().enumerate() {
        map.insert(ArrayKey::Int(idx as i64), *handle);
    }
    map
}

fn next_int_key(map: &IndexMap<ArrayKey, Handle>) -> i64 {
    map.keys()
        .filter_map(|key| match key {
            ArrayKey::Int(i) => Some(*i),
            _ => None,
        })
        .max()
        .map(|max| max + 1)
        .unwrap_or(0)
}

fn replacement_values(vm: &VM, replacement: Option<Handle>) -> Vec<Handle> {
    let Some(handle) = replacement else {
        return Vec::new();
    };

    match &vm.arena.get(handle).value {
        Val::Array(arr) => arr.map.values().copied().collect(),
        _ => vec![handle],
    }
}

fn merge_recursive_values(vm: &mut VM, left: Handle, right: Handle) -> Result<Handle, String> {
    let left_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(left).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => {
            let values = vec![left, right];
            let map = list_to_map(&values);
            return Ok(vm
                .arena
                .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())));
        }
    };

    let right_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(right).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => {
            let mut values: Vec<Handle> = left_entries.iter().map(|(_, v)| *v).collect();
            values.push(right);
            let map = list_to_map(&values);
            return Ok(vm
                .arena
                .alloc(Val::Array(crate::core::value::ArrayData::from(map).into())));
        }
    };

    let mut out = IndexMap::new();
    let mut next_int_key = 0i64;

    for (key, value_handle) in left_entries {
        match key {
            ArrayKey::Int(_) => {
                out.insert(ArrayKey::Int(next_int_key), value_handle);
                next_int_key += 1;
            }
            ArrayKey::Str(s) => {
                out.insert(ArrayKey::Str(s.clone()), value_handle);
            }
        }
    }

    for (key, value_handle) in right_entries {
        match key {
            ArrayKey::Int(_) => {
                out.insert(ArrayKey::Int(next_int_key), value_handle);
                next_int_key += 1;
            }
            ArrayKey::Str(s) => {
                let key_clone = ArrayKey::Str(s.clone());
                if let Some(existing_handle) = out.get(&key_clone).copied() {
                    let merged = merge_recursive_values(vm, existing_handle, value_handle)?;
                    out.insert(key_clone, merged);
                } else {
                    out.insert(key_clone, value_handle);
                }
            }
        }
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

fn replace_recursive_values(vm: &mut VM, left: Handle, right: Handle) -> Result<Handle, String> {
    let left_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(left).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Ok(right),
    };

    let right_entries: Vec<(ArrayKey, Handle)> = match &vm.arena.get(right).value {
        Val::Array(arr) => arr.map.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        _ => return Ok(right),
    };

    let mut out = IndexMap::new();
    for (key, value_handle) in left_entries {
        out.insert(key, value_handle);
    }

    for (key, value_handle) in right_entries {
        if let Some(existing_handle) = out.get(&key).copied() {
            if let (Val::Array(_), Val::Array(_)) = (
                &vm.arena.get(existing_handle).value,
                &vm.arena.get(value_handle).value,
            ) {
                let merged = replace_recursive_values(vm, existing_handle, value_handle)?;
                out.insert(key, merged);
                continue;
            }
        }
        out.insert(key, value_handle);
    }

    Ok(vm
        .arena
        .alloc(Val::Array(crate::core::value::ArrayData::from(out).into())))
}

pub fn php_deka_array(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array(vm, args)
}

pub fn php_deka_array_all(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_all(vm, args)
}

pub fn php_deka_array_any(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_any(vm, args)
}

pub fn php_deka_array_change_key_case(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_change_key_case(vm, args)
}

pub fn php_deka_array_chunk(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_chunk(vm, args)
}

pub fn php_deka_array_column(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_column(vm, args)
}

pub fn php_deka_array_combine(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_combine(vm, args)
}

pub fn php_deka_array_count_values(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_count_values(vm, args)
}

pub fn php_deka_array_diff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_diff(vm, args)
}

pub fn php_deka_array_diff_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_diff_assoc(vm, args)
}

pub fn php_deka_array_diff_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_diff_key(vm, args)
}

pub fn php_deka_array_diff_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_diff_uassoc(vm, args)
}

pub fn php_deka_array_diff_ukey(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_diff_ukey(vm, args)
}

pub fn php_deka_array_fill(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_fill(vm, args)
}

pub fn php_deka_array_fill_keys(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_fill_keys(vm, args)
}

pub fn php_deka_array_filter(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_filter(vm, args)
}

pub fn php_deka_array_find(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_find(vm, args)
}

pub fn php_deka_array_find_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_find_key(vm, args)
}

pub fn php_deka_array_first(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_first(vm, args)
}

pub fn php_deka_array_flip(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_flip(vm, args)
}

pub fn php_deka_array_intersect(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_intersect(vm, args)
}

pub fn php_deka_array_intersect_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_intersect_assoc(vm, args)
}

pub fn php_deka_array_intersect_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_intersect_key(vm, args)
}

pub fn php_deka_array_intersect_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_intersect_uassoc(vm, args)
}

pub fn php_deka_array_intersect_ukey(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_intersect_ukey(vm, args)
}

pub fn php_deka_array_is_list(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_is_list(vm, args)
}

pub fn php_deka_array_key_first(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_key_first(vm, args)
}

pub fn php_deka_array_key_last(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_key_last(vm, args)
}

pub fn php_deka_array_last(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_last(vm, args)
}

pub fn php_deka_array_map(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_map(vm, args)
}

pub fn php_deka_array_merge(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_merge(vm, args)
}

pub fn php_deka_array_merge_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_merge_recursive(vm, args)
}

pub fn php_deka_array_multisort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_multisort(vm, args)
}

pub fn php_deka_array_pad(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_pad(vm, args)
}

pub fn php_deka_array_product(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_product(vm, args)
}

pub fn php_deka_array_reduce(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_reduce(vm, args)
}

pub fn php_deka_array_replace(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_replace(vm, args)
}

pub fn php_deka_array_replace_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_replace_recursive(vm, args)
}

pub fn php_deka_array_reverse(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_reverse(vm, args)
}

pub fn php_deka_array_search(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_search(vm, args)
}

pub fn php_deka_array_shift(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_shift(vm, args)
}

pub fn php_deka_array_slice(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_slice(vm, args)
}

pub fn php_deka_array_splice(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_splice(vm, args)
}

pub fn php_deka_array_sum(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_sum(vm, args)
}

pub fn php_deka_array_rand(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_rand(vm, args)
}

pub fn php_deka_array_udiff(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_udiff(vm, args)
}

pub fn php_deka_array_udiff_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_udiff_assoc(vm, args)
}

pub fn php_deka_array_udiff_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_udiff_uassoc(vm, args)
}

pub fn php_deka_array_uintersect(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_uintersect(vm, args)
}

pub fn php_deka_array_uintersect_assoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_uintersect_assoc(vm, args)
}

pub fn php_deka_array_uintersect_uassoc(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_uintersect_uassoc(vm, args)
}

pub fn php_deka_array_unique(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_unique(vm, args)
}

pub fn php_deka_array_walk(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_walk(vm, args)
}

pub fn php_deka_array_walk_recursive(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_array_walk_recursive(vm, args)
}

pub fn php_deka_arsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_arsort(vm, args)
}

pub fn php_deka_asort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_asort(vm, args)
}

pub fn php_deka_compact(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_compact(vm, args)
}

pub fn php_deka_extract(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_extract(vm, args)
}

pub fn php_deka_key(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_key(vm, args)
}

pub fn php_deka_key_exists(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_key_exists(vm, args)
}

pub fn php_deka_krsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_krsort(vm, args)
}

pub fn php_deka_natcasesort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_natcasesort(vm, args)
}

pub fn php_deka_natsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_natsort(vm, args)
}

pub fn php_deka_pos(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_pos(vm, args)
}

pub fn php_deka_prev(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_prev(vm, args)
}

pub fn php_deka_range(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_range(vm, args)
}

pub fn php_deka_rsort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_rsort(vm, args)
}

pub fn php_deka_shuffle(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_shuffle(vm, args)
}

pub fn php_deka_sort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_sort(vm, args)
}

pub fn php_deka_uasort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_uasort(vm, args)
}

pub fn php_deka_uksort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_uksort(vm, args)
}

pub fn php_deka_usort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_usort(vm, args)
}

pub fn php_deka_ksort(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_ksort(vm, args)
}

pub fn php_deka_current(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_current(vm, args)
}

pub fn php_deka_next(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_next(vm, args)
}

pub fn php_deka_reset(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_reset(vm, args)
}

pub fn php_deka_end(vm: &mut VM, args: &[Handle]) -> Result<Handle, String> {
    php_end(vm, args)
}
