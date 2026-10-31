<?php
// ReflectionExtension: a loaded module by any case, what it declares in
// PHP's order, its section; getExtension(); phpversion($extension);
// get_extension_funcs(); ReflectionZendExtension (there are none).

$json = new ReflectionExtension('JSON');
var_dump($json, $json->getName(), $json->getVersion(), $json->isPersistent(), $json->isTemporary());
$json->__construct('json');
var_dump($json);

foreach (["nope", "foo\0bar"] as $name) {
    try {
        new ReflectionExtension($name);
    } catch (ReflectionException $e) {
        echo get_class($e), ': ', str_replace("\0", '\0', $e->getMessage()), "\n";
    }
}
try {
    new ReflectionZendExtension('Xdebug');
} catch (ReflectionException $e) {
    echo $e->getMessage(), "\n";
}

foreach (['dom', 'SPL', 'session', 'standard', 'json', 'ctype'] as $ext) {
    echo $ext, ': ', json_encode((new ReflectionExtension($ext))->getDependencies()), "\n";
}

$functions = $json->getFunctions();
var_dump(array_keys($functions), $functions['json_encode'] instanceof ReflectionFunction, $functions['json_encode']->getName());
var_dump(array_keys((new ReflectionExtension('ctype'))->getFunctions()));
var_dump($json->getConstants()['JSON_PRETTY_PRINT'], count($json->getConstants()));
var_dump((new ReflectionExtension('standard'))->getConstants()['CONNECTION_NORMAL']);
var_dump($json->getClassNames(), (new ReflectionExtension('ctype'))->getClassNames());
$classes = (new ReflectionExtension('tokenizer'))->getClasses();
var_dump(array_keys($classes), $classes['PhpToken']->getName());
var_dump($json->getINIEntries());

// A module's directives in the order it registered them, by name once
// ini_get_all() has sorted PHP's table.
ini_set('date.default_latitude', '12.5');
$date = new ReflectionExtension('date');
var_dump($date->getINIEntries());
ini_get_all('date');
var_dump(array_keys($date->getINIEntries()));

foreach (['ctype', 'json', 'Reflection', 'tokenizer', 'SPL', 'filter', 'session'] as $ext) {
    ob_start();
    var_dump((new ReflectionExtension($ext))->info());
    echo ob_get_clean();
}

var_dump((new ReflectionClass('JsonException'))->getExtension() instanceof ReflectionExtension);
var_dump((new ReflectionClass('ArrayObject'))->getExtension()->getName());
var_dump((new ReflectionFunction('json_encode'))->getExtension()->getName());
var_dump((new ReflectionFunction('strlen'))->getExtension()->getName());
function user_function() {}
var_dump((new ReflectionFunction('user_function'))->getExtension());
class UserClass {}
var_dump((new ReflectionClass('UserClass'))->getExtension());

var_dump(phpversion() === PHP_VERSION, phpversion('json'), phpversion('JSON'), phpversion('Core'), phpversion('nope'));
var_dump(get_extension_funcs('json'), get_extension_funcs('ctype') === array_keys((new ReflectionExtension('ctype'))->getFunctions()));
var_dump(get_extension_funcs('Reflection'), get_extension_funcs('nope'), get_extension_funcs('zend') === get_extension_funcs('Core'));

// The module, its directives, constants and functions (its number is its
// place in the process's module list; p8's ReflectionClass has no
// __toString() to list its classes with).
echo preg_replace(['/ extension #\d+ /', '/\n  - Classes \[\d+\] \{.*\n  \}\n/s'], [' extension #N ', ''], (string) $json);
