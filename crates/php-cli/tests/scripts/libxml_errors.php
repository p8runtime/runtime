<?php
// libxml: errors are warnings, or with internal errors on a list; the last one is kept either way.
var_dump(extension_loaded('libxml'), libxml_use_internal_errors(), libxml_get_last_error(), libxml_get_errors());
$d = new DOMDocument;
var_dump($d->loadXML('<a><b></a>'));
$e = libxml_get_last_error(); var_dump($e);
var_dump(libxml_use_internal_errors(true), libxml_use_internal_errors(null), libxml_use_internal_errors());
var_dump($d->loadXML('<a><b></a>'), count(libxml_get_errors()));
var_dump(simplexml_load_string('<x>'), libxml_get_errors());
libxml_clear_errors(); var_dump(libxml_get_errors(), libxml_get_last_error());
var_dump($d->loadXML('<a></b>'), libxml_use_internal_errors(false), libxml_get_errors(), libxml_get_last_error() !== false);
var_dump(libxml_disable_entity_loader(true), libxml_disable_entity_loader(false));
var_dump(libxml_get_external_entity_loader(), libxml_set_external_entity_loader(fn() => null), is_callable(libxml_get_external_entity_loader()));
