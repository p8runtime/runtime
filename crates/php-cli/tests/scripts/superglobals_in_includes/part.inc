<?php
// Code of an included file: its superglobals are the global ones, whatever
// scope included it.
echo $where, ': ', is_array($_SERVER) ? 'array' : 'not an array', ', ', isset($_GET) ? 'GET set' : 'GET unset', "\n";
$_GET['seen'][] = $where;
