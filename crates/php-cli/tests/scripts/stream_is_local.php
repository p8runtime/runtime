<?php
foreach (['/etc/passwd', 'file:///x', 'php://memory', 'http://x', 'HTTPS://x', 'ftp://x', 'data:text/plain,hi', 'data://text/plain,hi', 'compress.zlib:///x', 'relative/path'] as $u) {
    echo $u, ' ', var_export(stream_is_local($u), true), "\n";
}
var_dump(stream_is_local(STDIN), stream_is_local(fopen('php://memory', 'r')));
