<?php
// A method whose last parameter is a by-reference variadic one
// (`&...$rest`) takes every argument past the declared ones by reference,
// however many there are: 70 variables, every one changed by the callee,
// the first time the call site runs and every time after.
class V {
    function all(&...$rest) { foreach ($rest as $i => &$r) { $r = "r$i"; } return count($rest); }
    function after($a, &$b, &...$rest) { $a = 'A'; $b = 'B'; foreach ($rest as $i => &$r) { $r .= "+$i"; } return count($rest); }
    function values($a, ...$rest) { $a = 'A'; foreach ($rest as $i => &$r) { $r = 'no'; } return count($rest); }
}
// The same names, nothing by reference.
class W {
    function all(...$rest) { foreach ($rest as $i => &$r) { $r = "w$i"; } return count($rest); }
    function after($a, $b, ...$rest) { $a = 'A'; $b = 'B'; foreach ($rest as $i => &$r) { $r .= "+$i"; } return count($rest); }
    function values(&$a, &...$rest) { $a = 'A'; foreach ($rest as $i => &$r) { $r = 'yes'; } return count($rest); }
}
function show($label, $n, $vars) {
    echo $label, ' ', $n, ': ', implode(' ', $vars), "\n";
}
function all_of($o) {
    $v0 = $v1 = $v2 = $v3 = $v4 = $v5 = $v6 = $v7 = $v8 = $v9 = $v10 = $v11 = $v12 = $v13 = $v14 = $v15 = $v16 = $v17 = $v18 = $v19 = $v20 = $v21 = $v22 = $v23 = $v24 = $v25 = $v26 = $v27 = $v28 = $v29 = $v30 = $v31 = $v32 = $v33 = $v34 = $v35 = $v36 = $v37 = $v38 = $v39 = $v40 = $v41 = $v42 = $v43 = $v44 = $v45 = $v46 = $v47 = $v48 = $v49 = $v50 = $v51 = $v52 = $v53 = $v54 = $v55 = $v56 = $v57 = $v58 = $v59 = $v60 = $v61 = $v62 = $v63 = $v64 = $v65 = $v66 = $v67 = $v68 = $v69 = 'v';
    $n = $o->all($v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69);
    show(get_class($o) . '::all', $n, [$v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69]);
}
function after_two($o) {
    $a = 'a'; $b = 'b';
    $v0 = $v1 = $v2 = $v3 = $v4 = $v5 = $v6 = $v7 = $v8 = $v9 = $v10 = $v11 = $v12 = $v13 = $v14 = $v15 = $v16 = $v17 = $v18 = $v19 = $v20 = $v21 = $v22 = $v23 = $v24 = $v25 = $v26 = $v27 = $v28 = $v29 = $v30 = $v31 = $v32 = $v33 = $v34 = $v35 = $v36 = $v37 = $v38 = $v39 = $v40 = $v41 = $v42 = $v43 = $v44 = $v45 = $v46 = $v47 = $v48 = $v49 = $v50 = $v51 = $v52 = $v53 = $v54 = $v55 = $v56 = $v57 = $v58 = $v59 = $v60 = $v61 = $v62 = $v63 = $v64 = $v65 = $v66 = $v67 = $v68 = $v69 = 'v';
    $n = $o->after($a, $b, $v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69);
    show(get_class($o) . '::after', $n, [$a, $b, $v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69]);
}
function values_of($o) {
    $a = 'a';
    $v0 = $v1 = $v2 = $v3 = $v4 = $v5 = $v6 = $v7 = $v8 = $v9 = $v10 = $v11 = $v12 = $v13 = $v14 = $v15 = $v16 = $v17 = $v18 = $v19 = $v20 = $v21 = $v22 = $v23 = $v24 = $v25 = $v26 = $v27 = $v28 = $v29 = $v30 = $v31 = $v32 = $v33 = $v34 = $v35 = $v36 = $v37 = $v38 = $v39 = $v40 = $v41 = $v42 = $v43 = $v44 = $v45 = $v46 = $v47 = $v48 = $v49 = $v50 = $v51 = $v52 = $v53 = $v54 = $v55 = $v56 = $v57 = $v58 = $v59 = $v60 = $v61 = $v62 = $v63 = $v64 = $v65 = $v66 = $v67 = $v68 = $v69 = 'v';
    $n = $o->values($a, $v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69);
    show(get_class($o) . '::values', $n, [$a, $v0, $v1, $v2, $v3, $v4, $v5, $v6, $v7, $v8, $v9, $v10, $v11, $v12, $v13, $v14, $v15, $v16, $v17, $v18, $v19, $v20, $v21, $v22, $v23, $v24, $v25, $v26, $v27, $v28, $v29, $v30, $v31, $v32, $v33, $v34, $v35, $v36, $v37, $v38, $v39, $v40, $v41, $v42, $v43, $v44, $v45, $v46, $v47, $v48, $v49, $v50, $v51, $v52, $v53, $v54, $v55, $v56, $v57, $v58, $v59, $v60, $v61, $v62, $v63, $v64, $v65, $v66, $v67, $v68, $v69]);
}
$v = new V;
$w = new W;
// Each site: its first run, a second (its cache has the method), then the
// other class (a miss), then the first again.
foreach ([$v, $v, $w, $v, $w, $w] as $o) {
    all_of($o);
    after_two($o);
    values_of($o);
}

// A site in a loop, the same object every time.
function looped($o) {
    for ($i = 0; $i < 3; $i++) {
        $p = $q = $r = "i$i";
        $o->all($p, $q, $r);
        echo "$p $q $r\n";
    }
}
looped($v);
looped($w);
