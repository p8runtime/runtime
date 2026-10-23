<?php
date_default_timezone_set('America/New_York');
$times = [0, 1, 86399, 951782400, 1104537600, 1230681600, 1719792000, 1700000000, -86400 * 400, 2147483647];
$all = '%a|%A|%b|%B|%c|%C|%d|%D|%e|%F|%g|%G|%h|%H|%I|%j|%k|%l|%m|%M|%n|%p|%P|%r|%R|%s|%S|%t|%T|%u|%U|%V|%w|%W|%x|%X|%y|%Y|%z|%Z|%%';
foreach ($times as $t) {
    echo @strftime($all, $t), "\n";
    echo @gmstrftime($all, $t), "\n";
}
echo @strftime('%_d %-d %05d %^a %#A %#Z %^B %10A %-10A %3Y %_5H %Ey %Od %q %', 1700000000), "\n";
var_dump(@strftime(''));
var_dump(@gmstrftime(''));
var_dump(strlen(@strftime('%Y')) == 4);
echo strftime('%Y', 0), "\n";
echo gmstrftime('%H', 3600), "\n";
