<?php
// Diagnostics as the server shows them (HTML, not the CLI's text).
echo "start\n";
echo $undefined;
echo 1 / 1, "\n";
trigger_error("custom <b>notice</b>", E_USER_NOTICE);
$what = $_GET['what'] ?? 'warn';
if ($what === 'fatal') {
    throw new Exception("boom <i>fatal</i>");
}
if ($what === 'exit') {
    header('X-Before: exit');
    exit("bye\n");
}
if ($what === 'late') {
    echo "output\n";
    header('X-Late: 1');
    echo json_encode(headers_sent($file, $line)), basename($file), $line, "\n";
}
