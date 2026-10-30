<?php
namespace App;
use Other\Thing;
enum Channel: string
{
    case Test = 'test';
    case Custom = Custom::class;
    case Imported = Thing::class;
}
class Custom {}
var_dump(Channel::Custom->value, Channel::Imported->value, Channel::from('App\Custom') === Channel::Custom);
function f($c = Custom::class) { return $c; }
var_dump(f());
