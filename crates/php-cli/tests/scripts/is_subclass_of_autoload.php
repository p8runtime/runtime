<?php
// is_subclass_of/is_a autoload a class given by name (WooCommerce checks its payment providers so).
spl_autoload_register(function ($c) { echo "autoload $c\n"; if ($c === 'Child') { eval('class Base {} class Child extends Base {}'); } if ($c === 'Other') { eval('class Other {}'); } });
var_dump(is_subclass_of('Child', 'Base'));
var_dump(is_subclass_of('Other', 'Base', false), is_subclass_of('Other', 'Base'));
var_dump(is_a('Missing', 'Base', true), is_subclass_of('Missing', 'Base'));
var_dump(is_subclass_of(new Child, 'Base'), is_a(new Child, 'Base'));
