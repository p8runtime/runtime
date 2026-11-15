<?php
// json_decode() objects whose members have numeric names (#567): each is
// reached by the name, as an int or a string, like any property write.
$o = json_decode("{\"3\":\"x\",\"a\":{\"9\":\"y\"},\"-1\":\"m\",\"07\":\"z\",\"\":\"e\"}");
$t = 3;
var_dump($o->{$t}, $o->{"3"}, $o->a->{9}, $o->{-1}, $o->{"07"});
$p = "3";
var_dump(isset($o->$p), isset($o->{7}), property_exists($o, "3"), property_exists($o, "9"));
$a = (array)$o;
var_dump(isset($a[3]), isset($a["07"]), array_keys($a));
var_dump(array_keys(get_object_vars($o)));
foreach ($o as $k => $v) {
    var_dump($k);
}
$o->{3} = "x2";
var_dump($o->{"3"}, count(get_object_vars($o)));
unset($o->{"3"});
var_dump(isset($o->{3}));
echo json_encode(json_decode("{\"1\":1,\"b\":{\"2\":[3]}}")), "\n";
var_dump(json_decode("{\"5\":{\"6\":true}}")->{5}->{"6"});
// foreach names a numeric property by a string, written either way.
$w = new stdClass();
$w->{5} = "five";
$w->b = "bee";
foreach ($w as $k => $v) {
    var_dump($k);
}
foreach ($w as $k => &$v) {
    var_dump($k);
    $v .= "!";
}
unset($v);
var_dump($w->{5}, array_keys((array)$w));
