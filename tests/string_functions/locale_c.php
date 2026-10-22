<?php
var_dump(localeconv());
foreach (['CODESET','D_T_FMT','D_FMT','T_FMT','T_FMT_AMPM','AM_STR','PM_STR','DAY_1','DAY_7','ABDAY_1','MON_1','MON_12','ABMON_1','RADIXCHAR','THOUSEP','YESEXPR','NOEXPR','CRNCYSTR','ERA','ALT_DIGITS'] as $c) echo $c, '=', constant($c), ' => ', var_export(nl_langinfo(constant($c)), true), "\n";
var_dump(strcoll("a", "b"), strcoll("b", "a"), strcoll("abc", "abc"), strcoll("a", "ab"), strcoll("B", "a"), strcoll("z", "a"));
foreach (['LC_CTYPE','LC_NUMERIC','LC_TIME','LC_COLLATE','LC_MONETARY','LC_MESSAGES','LC_ALL'] as $c) echo $c, '=', constant($c), "\n";
