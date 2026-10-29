<?php
// stream_resolve_include_path(): where an include would find a name, as a
// canonical path: the include_path, then the running file's directory; any
// existing path counts ("" names the first include_path directory).
var_dump(stream_resolve_include_path(basename(__FILE__)) === realpath(__FILE__));
var_dump(stream_resolve_include_path('no-such-file.php'));
set_include_path(__DIR__);
var_dump(stream_resolve_include_path(basename(__FILE__)) === realpath(__FILE__));
var_dump(stream_resolve_include_path('') === realpath(__DIR__));
var_dump(stream_resolve_include_path('./' . basename(__FILE__)));
