<?php
fwrite(STDOUT, "1 STDOUT after fflush\n"); fflush(STDOUT);
fwrite(STDERR, "2 STDERR\n");
$o = fopen('php://stdout', 'w'); fwrite($o, "3 php://stdout after fflush\n"); fflush($o);
fwrite(STDERR, "4 STDERR\n");
echo "5 echo\n"; flush();
fwrite(STDERR, "6 STDERR\n");
echo "7 echo without flush\n";
fwrite(STDERR, "8 STDERR\n");
