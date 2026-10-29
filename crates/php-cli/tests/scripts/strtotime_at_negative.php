<?php
// strtotime("@N"): a timestamp, before 1970 too (a sign after the @).
date_default_timezone_set("GMT");
foreach (["@1", "@-1", "@-2147483647", "@2147483647", "@+5", "@-5 +1 day"] as $s) {
    var_dump(strtotime($s));
}
echo date("Y-m-d H:i:s", strtotime("@-86400")), "\n";
