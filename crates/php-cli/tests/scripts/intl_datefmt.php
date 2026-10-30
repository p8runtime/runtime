<?php
// IntlDateFormatter: CLDR's date/time styles, ICU's pattern letters, time
// zones (IANA and custom offsets), DateTime arguments, parsing.
date_default_timezone_set('Europe/Berlin');
$ts = 1700000000; // 2023-11-14 22:13:20 UTC
$styles = [IntlDateFormatter::FULL, IntlDateFormatter::LONG, IntlDateFormatter::MEDIUM, IntlDateFormatter::SHORT, IntlDateFormatter::NONE];
foreach (['en_US', 'de_DE', 'en_GB'] as $l) {
    foreach ($styles as $d) {
        foreach ($styles as $t) {
            if ($d == IntlDateFormatter::NONE && $t == IntlDateFormatter::NONE) {
                continue;
            }
            $f = new IntlDateFormatter($l, $d, $t, 'America/New_York');
            echo "$l $d $t | ", $f->format($ts), ' | ', $f->getPattern(), "\n";
        }
    }
}

$f = new IntlDateFormatter('en_US', IntlDateFormatter::NONE, IntlDateFormatter::NONE, 'UTC', IntlDateFormatter::GREGORIAN, "G GGGG y yy yyyy u Q q M MM MMM MMMM MMMMM L LLL LLLL w ww W d dd D DDD F E EEEE EEEEE EEEEEE e ee eee c ccc a h hh H HH k K m mm s ss S SS SSS A z zzzz Z ZZZZZ O OOOO X XX XXX x xxx Y YY 'quoted' ''");
echo $f->format($ts), "\n";
foreach (['Asia/Kolkata', 'America/Los_Angeles', 'Australia/Sydney', 'GMT+2', 'Etc/GMT-3'] as $tz) {
    $f = new IntlDateFormatter('en_US', IntlDateFormatter::NONE, IntlDateFormatter::NONE, $tz, IntlDateFormatter::GREGORIAN, 'yyyy-MM-dd HH:mm z | zzzz | Z | ZZZZ | O | OOOO | VV | X | xxx');
    echo $tz, ': ', $f->format($ts), ' | ', $f->getTimeZoneId(), "\n";
}

$f = new IntlDateFormatter('en_US', IntlDateFormatter::MEDIUM, IntlDateFormatter::MEDIUM);
var_dump($f->getTimeZoneId(), $f->getDateType(), $f->getTimeType(), $f->getCalendar(), $f->isLenient());
var_dump($f->format(new DateTime('2020-02-29 13:05:09', new DateTimeZone('Asia/Tokyo'))));
var_dump($f->format(['tm_sec' => 1, 'tm_min' => 2, 'tm_hour' => 3, 'tm_mday' => 4, 'tm_mon' => 5, 'tm_year' => 99, 'tm_wday' => 0, 'tm_yday' => 0, 'tm_isdst' => 0]));
var_dump($f->format(1.5e9), $f->format('1500000000'), $f->format('abc'), intl_get_error_message());
var_dump(IntlDateFormatter::formatObject(new DateTime('2020-02-29 13:05:09', new DateTimeZone('Asia/Tokyo')), null, 'en_US'));
var_dump(IntlDateFormatter::formatObject(new DateTimeImmutable('2020-02-29 13:05:09', new DateTimeZone('UTC')), 'yyyy-MM-dd HH:mm z', 'en_US'));
var_dump(IntlDateFormatter::formatObject(new DateTime('2020-02-29 13:05:09'), [IntlDateFormatter::SHORT, IntlDateFormatter::NONE], 'fr'));

$f = new IntlDateFormatter('en_US', IntlDateFormatter::SHORT, IntlDateFormatter::SHORT, 'UTC');
var_dump($f->parse('11/14/23, 10:13 PM'), $f->parse('garbage'), $f->getErrorMessage());
var_dump($f->setPattern('yyyy-MM-dd'), $f->getPattern(), $f->parse('2023-11-14'));
var_dump($f->setTimeZone('Asia/Tokyo'), $f->getTimeZoneId(), $f->setTimeZone('Nope/Nope'), $f->getErrorMessage());
var_dump(get_class($f->getTimeZone()), $f->getTimeZone()->getID());
$f = new IntlDateFormatter('en_US', IntlDateFormatter::SHORT, IntlDateFormatter::SHORT, new DateTimeZone('+05:30'));
var_dump($f->getTimeZoneId(), $f->format(0));
$f = new IntlDateFormatter('en_US', IntlDateFormatter::FULL, IntlDateFormatter::FULL, 'UTC', IntlDateFormatter::TRADITIONAL);
var_dump($f->format(0), $f->getCalendar());

var_dump(datefmt_create('en_US', 99, 99), intl_get_error_message());
try {
    new IntlDateFormatter('en_US', 99, 99);
} catch (IntlException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    new IntlDateFormatter('en_US', IntlDateFormatter::SHORT, IntlDateFormatter::SHORT, 'Invalid/Zone');
} catch (IntlException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

$tz = IntlTimeZone::createTimeZone('Europe/Paris');
var_dump($tz->getID(), $tz->getRawOffset(), $tz->useDaylightTime(), $tz->getDSTSavings(), get_class($tz->toDateTimeZone()), $tz->toDateTimeZone()->getName());
var_dump(IntlTimeZone::createTimeZone('GMT+01:00')->getID(), IntlTimeZone::getGMT()->getID(), IntlTimeZone::createTimeZone('Not/AZone')->getID());
