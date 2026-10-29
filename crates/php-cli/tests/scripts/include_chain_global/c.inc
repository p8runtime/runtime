<?php
// Like shortcodes.php: sets the variable, then a function fills the global.
$tags = [];
function add_tag($t) { global $tags; $tags[$t] = true; }
add_tag('gallery');
add_tag('caption');
