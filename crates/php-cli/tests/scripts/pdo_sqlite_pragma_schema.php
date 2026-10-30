<?php
$db = new PDO('sqlite::memory:');
$db->exec('create table t (id integer primary key, name text not null default \'x\')');
foreach ($db->query("select name, \"notnull\", dflt_value, pk from pragma_table_xinfo('t', 'main') order by cid") as $r) {
    echo implode('|', [$r['name'], $r['notnull'], $r['dflt_value'] ?? 'NULL', $r['pk']]), "\n";
}
var_dump(count($db->query("select * from pragma_index_list('t', 'main')")->fetchAll()));
var_dump($db->query("select 'pragma_x(1, ''main'')' as s")->fetchColumn());
