<?php
date_default_timezone_set("America/Sao_Paulo");
$d = new DateTime("2021-03-04 05:06:07.123456");
$s = serialize($d); echo $s, "\n"; var_dump(unserialize($s));
$z = new DateTimeZone("Europe/Paris"); echo serialize($z), "\n"; var_dump(unserialize(serialize($z)));
$i = new DateTimeImmutable("2020-01-01 00:00:00+02:00"); var_dump(unserialize(serialize($i)));
