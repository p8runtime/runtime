<?php
parse_str('a=1&b=2', $output);
ksort($output);
print_r($output);
