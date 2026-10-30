<?php
// An array known at compile time through __DIR__ / __FILE__ concatenations is
// one constant (Composer's class map has 15,000 such entries): larger than a
// function's register space, it loads whole.
$file = tempnam(sys_get_temp_dir(), 'map');
$src = "<?php\nclass M {\n    public static \$map = array(\n";
for ($i = 0; $i < 15000; $i++) { $src .= "        'K$i' => __DIR__ . '/..' . '/p/$i.php',\n"; }
file_put_contents($file, $src . "    );\n}\nreturn [count(M::\$map), M::\$map['K14999'] === __DIR__ . '/../p/14999.php', basename(M::\$map['K7'])];");
var_dump(include $file);
unlink($file);
$f = fn() => [__LINE__, -1, 'a' . 'b' . 3];
var_dump($f());
