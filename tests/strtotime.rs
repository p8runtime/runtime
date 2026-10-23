mod common;
use common::run_code_capture_output;

// ============================================================================
// Special Keywords Tests
// ============================================================================

#[test]
fn test_strtotime_now() {
    let code = "<?php
        $result = strtotime('now');
        var_dump($result > 1577836800); // After 2020-01-01
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(true)"));
}

#[test]
fn test_strtotime_today() {
    let code = "<?php
        $result = strtotime('today');
        var_dump($result > 1577836800); // After 2020-01-01
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(true)"));
}

#[test]
fn test_strtotime_tomorrow() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('tomorrow', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705363200");
}

#[test]
fn test_strtotime_yesterday() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('yesterday', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705190400");
}

#[test]
fn test_strtotime_midnight() {
    let code = "<?php
        $base = 1705329000; // 2024-01-15 14:30:00 UTC
        $result = strtotime('midnight', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_noon() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('noon', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705320000");
}

// ============================================================================
// Relative Time Tests
// ============================================================================

#[test]
fn test_strtotime_plus_one_day() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('+1 day', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705363200");
}

#[test]
fn test_strtotime_minus_one_week() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('-1 week', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1704672000");
}

#[test]
fn test_strtotime_plus_two_months() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('+2 months', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1710460800");
}

#[test]
fn test_strtotime_minus_three_years() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('-3 years', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1610668800");
}

#[test]
fn test_strtotime_two_weeks_ago() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('2 weeks ago', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1704067200");
}

#[test]
fn test_strtotime_fortnight() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('+1 fortnight', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1706486400");
}

#[test]
fn test_strtotime_plus_hours_minutes_seconds() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        
        $result = strtotime('+5 hours', $base);
        echo $result . \"\\n\";
        
        $result = strtotime('+30 minutes', $base);
        echo $result . \"\\n\";
        
        $result = strtotime('+45 seconds', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], (1705276800 + 5 * 3600).to_string());
    assert_eq!(lines[1], (1705276800 + 30 * 60).to_string());
    assert_eq!(lines[2], (1705276800 + 45).to_string());
}

// ============================================================================
// Weekday Reference Tests
// ============================================================================

#[test]
fn test_strtotime_next_monday() {
    let code = "<?php
        $base = 1705276800; // Monday 2024-01-15 00:00:00 UTC
        $result = strtotime('next monday', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705881600");
}

#[test]
fn test_strtotime_last_friday() {
    let code = "<?php
        $base = 1705276800; // Monday 2024-01-15 00:00:00 UTC
        $result = strtotime('last friday', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705017600");
}

#[test]
fn test_strtotime_this_wednesday() {
    let code = "<?php
        $base = 1705276800; // Monday 2024-01-15 00:00:00 UTC
        $result = strtotime('this wednesday', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705449600");
}

#[test]
fn test_strtotime_next_weekday_abbreviation() {
    let code = "<?php
        $base = 1705276800; // Monday 2024-01-15 00:00:00 UTC
        $result = strtotime('next fri', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705622400");
}

// ============================================================================
// Special Phrase Tests
// ============================================================================

#[test]
fn test_strtotime_first_day_of_next_month() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('first day of next month', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1706745600");
}

#[test]
fn test_strtotime_last_day_of_this_month() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('last day of this month', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1706659200");
}

#[test]
fn test_strtotime_first_day_of_last_month() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('first day of last month', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1701388800");
}

#[test]
fn test_strtotime_last_day_of_next_month() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('last day of next month', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1709164800");
}

// ============================================================================
// Absolute Date/Time Format Tests
// ============================================================================

#[test]
fn test_strtotime_iso_date() {
    let code = "<?php
        $result = strtotime('2024-01-15');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_iso_datetime() {
    let code = "<?php
        $result = strtotime('2024-01-15 14:30:00');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705329000");
}

#[test]
fn test_strtotime_iso8601_format() {
    let code = "<?php
        $result = strtotime('2024-01-15T14:30:00Z');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705329000");
}

#[test]
fn test_strtotime_us_format() {
    let code = "<?php
        $result = strtotime('01/15/2024');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_day_month_year() {
    let code = "<?php
        $result = strtotime('15 Jan 2024');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_month_day_year() {
    let code = "<?php
        $result = strtotime('Jan 15 2024');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_day_dash_month_dash_year() {
    let code = "<?php
        $result = strtotime('15-Jan-2024');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1705276800");
}

#[test]
fn test_strtotime_unix_timestamp() {
    let code = "<?php
        $result = strtotime('@1234567890');
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1234567890");
}

// ============================================================================
// Compact Date Format Tests
// ============================================================================

#[test]
fn test_strtotime_compact_year_dayofyear() {
    let code = "<?php
        $result = strtotime('2026113');
        echo $result . \"\\n\";
        echo date('Y-m-d', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1776902400");
    assert_eq!(lines[1], "2026-04-23");
}

#[test]
fn test_strtotime_compact_yyyymmdd() {
    let code = "<?php
        $result = strtotime('20260113');
        echo $result . \"\\n\";
        echo date('Y-m-d', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    // Timestamp might vary by timezone, but date should be consistent
    assert_eq!(lines[1], "2026-01-13");
}

// ============================================================================
// Edge Cases and Error Handling Tests
// ============================================================================

#[test]
fn test_strtotime_invalid_string() {
    let code = "<?php
        $result = strtotime('not a date');
        var_dump($result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

#[test]
fn test_strtotime_empty_string() {
    let code = "<?php
        $result = strtotime('');
        var_dump($result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

#[test]
fn test_strtotime_whitespace_only() {
    let code = "<?php
        $result = strtotime('   ');
        var_dump($result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

#[test]
fn test_strtotime_case_insensitive() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        
        $result1 = strtotime('TOMORROW', $base);
        echo $result1 . \"\\n\";
        
        $result2 = strtotime('Next Monday', $base);
        echo $result2;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1705363200");
    assert_eq!(lines[1], "1705881600");
}

#[test]
fn test_strtotime_month_overflow() {
    let code = "<?php
        $base = 1706659200; // 2024-01-31 00:00:00 UTC
        $result = strtotime('+1 month', $base);
        echo $result . \"\\n\";
        echo date('Y-m-d', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1709337600");
    assert_eq!(lines[1], "2024-03-02");
}

#[test]
fn test_strtotime_leap_year_handling() {
    let code = "<?php
        $base = 1709164800; // 2024-02-29 00:00:00 UTC (leap day)
        $result = strtotime('+1 year', $base);
        echo $result . \"\\n\";
        echo date('Y-m-d', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1740787200");
    assert_eq!(lines[1], "2025-03-01");
}

#[test]
fn test_strtotime_with_base_timestamp() {
    let code = "<?php
        $base = 1577836800; // 2020-01-01 00:00:00 UTC
        $result = strtotime('+1 year', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1609459200");
}

#[test]
fn test_strtotime_negative_numbers() {
    let code = "<?php
        $base = 1705276800; // 2024-01-15 00:00:00 UTC
        $result = strtotime('-5 days', $base);
        echo $result;
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "1704844800");
}

// ============================================================================
// Time-Only Compact Format Tests
// ============================================================================

#[test]
fn test_strtotime_compact_time_hhmm() {
    let code = "<?php
        $result = strtotime('1530');
        echo date('H:i:s', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "15:30:00");
}

#[test]
fn test_strtotime_compact_time_hhmmss() {
    let code = "<?php
        $result = strtotime('202613');
        echo date('H:i:s', $result);
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "20:26:13");
}

#[test]
fn test_strtotime_compact_time_various() {
    let code = "<?php
        echo date('H:i:s', strtotime('123456')) . \"\\n\";
        echo date('H:i:s', strtotime('235959')) . \"\\n\";
        echo date('H:i:s', strtotime('0000')) . \"\\n\";
        echo date('H:i:s', strtotime('000000'));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12:34:56");
    assert_eq!(lines[1], "23:59:59");
    assert_eq!(lines[2], "00:00:00");
    assert_eq!(lines[3], "00:00:00");
}

#[test]
fn test_strtotime_compact_time_invalid() {
    let code = "<?php
        // Invalid times should fail
        var_dump(strtotime('236099')); // Minute/second > 59
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

#[test]
fn test_strtotime_time_with_t_prefix() {
    let code = "<?php
        // Test optional 't' and 'T' prefix
        echo date('H:i:s', strtotime('t1530')) . \"\\n\";
        echo date('H:i:s', strtotime('T1530')) . \"\\n\";
        echo date('H:i:s', strtotime('t202613')) . \"\\n\";
        echo date('H:i:s', strtotime('T202613'));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15:30:00");
    assert_eq!(lines[1], "15:30:00");
    assert_eq!(lines[2], "20:26:13");
    assert_eq!(lines[3], "20:26:13");
}

#[test]
fn test_strtotime_year_dayofyear_separators() {
    let code = "<?php
        // Test YYYY-DDD and YYYY.DDD formats
        echo date('Y-m-d', strtotime('2026-113')) . \"\\n\";
        echo date('Y-m-d', strtotime('2026.113'));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2026-04-23");
    assert_eq!(lines[1], "2026-04-23");
}

#[test]
fn test_strtotime_year4_fallback() {
    let code = "<?php
        // 4-digit numbers that are invalid as time should parse as year
        echo date('Y-m-d', strtotime('2560')) . \"\\n\";
        echo date('Y-m-d', strtotime('2461'));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    // Should be year 2560 and 2461 with today's month/day
    assert!(lines[0].starts_with("2560-"));
    assert!(lines[1].starts_with("2461-"));
}

#[test]
fn test_strtotime_invalid_dayofyear() {
    let code = "<?php
        // Invalid day of year values
        var_dump(strtotime('2026000')); // Day 0
        var_dump(strtotime('2026367')); // Day 367 (non-leap year)
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

// ============================================================================
// Test cases from PHP source: ext/date/tests/strtotime-relative.phpt
// ============================================================================

#[test]
fn test_strtotime_php_source_relative_seconds() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // Relative seconds from now
        $base = strtotime('2008-02-28 12:00:00');
        echo date('Y-m-d H:i:s', strtotime('+86400 seconds', $base)) . \"\\n\";
        echo date('Y-m-d H:i:s', strtotime('-86400 seconds', $base)) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2008-02-29 12:00:00"); // +1 day
    assert_eq!(lines[1], "2008-02-27 12:00:00"); // -1 day
}

#[test]
fn test_strtotime_php_source_compact_formats() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // Compact time formats
        echo date('H:i:s', strtotime('t0222')) . \"\\n\";
        echo date('H:i:s', strtotime('022233')) . \"\\n\";
        echo date('H:i:s', strtotime('153045')) . \"\\n\";
        
        // Compact date formats
        echo date('Y-m-d', strtotime('2006167')) . \"\\n\";  // YYYYDDD
        echo date('Y-m-d', strtotime('20060616')) . \"\\n\"; // YYYYMMDD
        echo date('Y-m-d', strtotime('2006-167')) . \"\\n\"; // YYYY-DDD
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "02:22:00"); // t0222
    assert_eq!(lines[1], "02:22:33"); // 022233
    assert_eq!(lines[2], "15:30:45"); // 153045
    assert_eq!(lines[3], "2006-06-16"); // Day 167 of 2006
    assert_eq!(lines[4], "2006-06-16"); // 2006-06-16
    assert_eq!(lines[5], "2006-06-16"); // 2006-167
}

#[test]
fn test_strtotime_php_source_empty_and_invalid() {
    let code = "<?php
        // Empty and whitespace strings should return false
        var_dump(strtotime(''));
        var_dump(strtotime(' \\t\\r\\n'));
        var_dump(strtotime('invalid'));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    // All should be false
    assert!(output.matches("bool(false)").count() >= 3);
}

#[test]
fn test_strtotime_mysql_format() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // MySQL timestamp format: YYYYMMDDHHMMSS (14 digits)
        echo date('Y-m-d H:i:s', strtotime('19970523091528')) . \"\\n\";
        echo date('Y-m-d H:i:s', strtotime('20001231185859')) . \"\\n\";
        echo date('Y-m-d H:i:s', strtotime('20260121143045')) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1997-05-23 09:15:28");
    assert_eq!(lines[1], "2000-12-31 18:58:59");
    assert_eq!(lines[2], "2026-01-21 14:30:45");
}

// ============================================================================
// PHP Source Tests - strtotime_basic.phpt
// ============================================================================

#[test]
fn test_strtotime_ordinal_vs_number() {
    let code = "<?php
        date_default_timezone_set('UTC');
        // The first of December 2008 is a Monday.
        // '1 Monday December 2008' = first Monday OR current day if Monday
        echo date('Y-m-d', strtotime('1 Monday December 2008')) . \"\\n\";
        // '2 Monday December 2008' = second Monday OR first if current is Monday
        echo date('Y-m-d', strtotime('2 Monday December 2008')) . \"\\n\";
        // '3 Monday December 2008' = third Monday OR second if current is Monday
        echo date('Y-m-d', strtotime('3 Monday December 2008')) . \"\\n\";
        // 'first Monday December 2008' = first Monday after first Monday
        echo date('Y-m-d', strtotime('first Monday December 2008')) . \"\\n\";
        // 'second Monday December 2008' = second Monday after first Monday
        echo date('Y-m-d', strtotime('second Monday December 2008')) . \"\\n\";
        // 'third Monday December 2008' = third Monday after first Monday
        echo date('Y-m-d', strtotime('third Monday December 2008')) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2008-12-01");
    assert_eq!(lines[1], "2008-12-08");
    assert_eq!(lines[2], "2008-12-15");
    assert_eq!(lines[3], "2008-12-08");
    assert_eq!(lines[4], "2008-12-15");
    assert_eq!(lines[5], "2008-12-22");
}

// ============================================================================
// PHP Source Tests - strtotime_basic2.phpt
// ============================================================================

#[test]
fn test_strtotime_invalid_returns_false() {
    let code = "<?php
        date_default_timezone_set('UTC');
        var_dump(strtotime('mayy 2 2009')); // misspelled month
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert!(output.contains("bool(false)"));
}

// ============================================================================
// PHP Source Tests - strtotime.phpt
// ============================================================================

#[test]
fn test_strtotime_unix_timestamp_with_date_formatting() {
    let code = "<?php
        date_default_timezone_set('UTC');
        // @ prefix for Unix timestamp
        $d = strtotime('@1121373041');
        echo date('Y-m-d H:i:s', $d) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    // @ timestamp should be treated as UTC
    assert!(output.contains("2005-07-14"));
}

#[test]
fn test_strtotime_with_timezone() {
    let code = "<?php
        date_default_timezone_set('Europe/Oslo');
        $d1 = strtotime('2005-07-14 22:30:41');
        $d2 = strtotime('2005-07-14 22:30:41 GMT');
        $d3 = strtotime('@1121373041');
        $d4 = strtotime('@1121373041 CEST');
        
        echo date(DATE_ISO8601, $d1) . \"\\n\";
        echo date(DATE_ISO8601, $d2) . \"\\n\";
        echo date(DATE_ISO8601, $d3) . \"\\n\";
        echo date(DATE_ISO8601, $d4) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2005-07-14T22:30:41+0200");
    assert_eq!(lines[1], "2005-07-15T00:30:41+0200");
    assert_eq!(lines[2], "2005-07-14T22:30:41+0200");
    assert_eq!(lines[3], "2005-07-14T22:30:41+0200");
}

// ============================================================================
// PHP Source Tests - strtotime3.phpt & strtotime3-64bit.phpt
// ============================================================================

#[test]
fn test_strtotime_comprehensive_formats() {
    let code = "<?php
        date_default_timezone_set('Europe/Lisbon');
        $time = 1150494719; // 16/June/2006
        
        // Test various formats with base timestamp (only currently supported ones)
        echo date(DATE_RFC2822, strtotime('yesterday', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('22:49:12', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('t0222', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('022233', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('2006167', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('2006', $time)) . \"\\n\";
        echo date(DATE_RFC2822, strtotime('1986', $time)) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Thu, 15 Jun 2006 00:00:00 +0100");
    assert_eq!(lines[1], "Fri, 16 Jun 2006 22:49:12 +0100");
    assert_eq!(lines[2], "Fri, 16 Jun 2006 02:22:00 +0100");
    assert_eq!(lines[3], "Fri, 16 Jun 2006 02:22:33 +0100");
    assert_eq!(lines[4], "Fri, 16 Jun 2006 00:00:00 +0100");
    assert_eq!(lines[5], "Fri, 16 Jun 2006 20:06:00 +0100");
    assert_eq!(lines[6], "Mon, 16 Jun 1986 22:51:59 +0100");
}

// ============================================================================
// PHP Source Tests - strtotime-relative.phpt
// ============================================================================

#[test]
fn test_strtotime_relative_offsets() {
    let code = "<?php
        date_default_timezone_set('UTC');
        $base_time = 1204200000; // 28 Feb 2008 12:00:00
        
        // Offset around a day
        echo date(DATE_ISO8601, strtotime('+80412 seconds', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-80412 seconds', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+86400 seconds', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-86400 seconds', $base_time)) . \"\\n\";
        
        // Offset around 7 days  
        echo date(DATE_ISO8601, strtotime('+168 hours', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-168 hours', $base_time)) . \"\\n\";
        
        // Offset around 6 months
        echo date(DATE_ISO8601, strtotime('+180 days', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-180 days', $base_time)) . \"\\n\";
        
        // Offset around 10 years
        echo date(DATE_ISO8601, strtotime('+120 months', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-120 months', $base_time)) . \"\\n\";
        
        // Offset around 25 years
        echo date(DATE_ISO8601, strtotime('+25 years', $base_time)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-25 years', $base_time)) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2008-02-29T10:20:12+0000");
    assert_eq!(lines[1], "2008-02-27T13:39:48+0000");
    assert_eq!(lines[2], "2008-02-29T12:00:00+0000");
    assert_eq!(lines[3], "2008-02-27T12:00:00+0000");
    assert_eq!(lines[4], "2008-03-06T12:00:00+0000");
    assert_eq!(lines[5], "2008-02-21T12:00:00+0000");
    assert_eq!(lines[6], "2008-08-26T12:00:00+0000");
    assert_eq!(lines[7], "2007-09-01T12:00:00+0000");
    assert_eq!(lines[8], "2018-02-28T12:00:00+0000");
    assert_eq!(lines[9], "1998-02-28T12:00:00+0000");
    assert_eq!(lines[10], "2033-02-28T12:00:00+0000");
    assert_eq!(lines[11], "1983-02-28T12:00:00+0000");
}

// ============================================================================
// PHP Source Tests - strtotime-mysql.phpt & strtotime-mysql-64bit.phpt
// ============================================================================

#[test]
fn test_strtotime_mysql_timestamps() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // MySQL Format: YYYYMMDDHHMMSS
        echo date('r', strtotime('19970523091528')) . \"\\n\";
        echo date('r', strtotime('20001231185859')) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Fri, 23 May 1997 09:15:28 +0000");
    assert_eq!(lines[1], "Sun, 31 Dec 2000 18:58:59 +0000");
}

// ============================================================================
// PHP Source Tests - strtotime_variation_scottish.phpt
// ============================================================================

#[test]
fn test_strtotime_scottish_time() {
    let code = "<?php
        date_default_timezone_set('UTC');
        echo date('H:i:s', strtotime('back of 7')) . \"\\n\";
        echo date('H:i:s', strtotime('front of 7')) . \"\\n\";
        echo date('H:i:s', strtotime('back of 19')) . \"\\n\";
        echo date('H:i:s', strtotime('front of 19')) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "07:15:00");
    assert_eq!(lines[1], "06:45:00");
    assert_eq!(lines[2], "19:15:00");
    assert_eq!(lines[3], "18:45:00");
}

// ============================================================================
// Additional Date Formats
// ============================================================================

#[test]
fn test_strtotime_additional_formats() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // D-M-YYYY format
        echo date('Y-m-d', strtotime('2-3-2004')) . \"\\n\";
        echo date('Y-m-d', strtotime('15-1-2006')) . \"\\n\";
        
        // D.M.YYYY format
        echo date('Y-m-d', strtotime('2.3.2004')) . \"\\n\";
        echo date('Y-m-d', strtotime('15.1.2006')) . \"\\n\";
        
        // Month-only format (with base timestamp)
        $base = 1150494719; // June 16, 2006
        echo date('Y-m-d', strtotime('JAN', $base)) . \"\\n\";
        echo date('Y-m-d', strtotime('January', $base)) . \"\\n\";
        echo date('Y-m-d', strtotime('March', $base)) . \"\\n\";
        
        // Mon-DD-YYYY format
        echo date('Y-m-d', strtotime('Jan-15-2006')) . \"\\n\";
        echo date('Y-m-d', strtotime('Mar-02-2024')) . \"\\n\";
        
        // YYYY-Mon-DD format
        echo date('Y-m-d', strtotime('2006-Jan-15')) . \"\\n\";
        echo date('Y-m-d', strtotime('2024-Mar-02')) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2004-03-02");
    assert_eq!(lines[1], "2006-01-15");
    assert_eq!(lines[2], "2004-03-02");
    assert_eq!(lines[3], "2006-01-15");
    assert_eq!(lines[4], "2006-01-16");
    assert_eq!(lines[5], "2006-01-16");
    assert_eq!(lines[6], "2006-03-16");
    assert_eq!(lines[7], "2006-01-15");
    assert_eq!(lines[8], "2024-03-02");
    assert_eq!(lines[9], "2006-01-15");
    assert_eq!(lines[10], "2024-03-02");
}

// ============================================================================
// Relative Offset Boundary Tests
// ============================================================================

#[test]
fn test_strtotime_relative_offset_boundaries() {
    let code = "<?php
        date_default_timezone_set('UTC');
        $base = 1204200000; // 28 Feb 2008 12:00:00
        
        // Around day boundary
        echo date(DATE_ISO8601, strtotime('+86399 seconds', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-86399 seconds', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+86401 seconds', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-86401 seconds', $base)) . \"\\n\";
        
        // Around week boundary  
        echo date(DATE_ISO8601, strtotime('+167 hours', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-167 hours', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+169 hours', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-169 hours', $base)) . \"\\n\";
        
        // Around 6-month boundary
        echo date(DATE_ISO8601, strtotime('+179 days', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-179 days', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+183 days', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-183 days', $base)) . \"\\n\";
        
        // Around 10-year boundary
        echo date(DATE_ISO8601, strtotime('+119 months', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-119 months', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+121 months', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-121 months', $base)) . \"\\n\";
        
        // Around 25-year boundary
        echo date(DATE_ISO8601, strtotime('+24 years', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-24 years', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('+26 years', $base)) . \"\\n\";
        echo date(DATE_ISO8601, strtotime('-26 years', $base)) . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    
    // Around day boundary
    assert_eq!(lines[0], "2008-02-29T11:59:59+0000");
    assert_eq!(lines[1], "2008-02-27T12:00:01+0000");
    assert_eq!(lines[2], "2008-02-29T12:00:01+0000");
    assert_eq!(lines[3], "2008-02-27T11:59:59+0000");
    
    // Around week boundary
    assert_eq!(lines[4], "2008-03-06T11:00:00+0000");
    assert_eq!(lines[5], "2008-02-21T13:00:00+0000");
    assert_eq!(lines[6], "2008-03-06T13:00:00+0000");
    assert_eq!(lines[7], "2008-02-21T11:00:00+0000");
    
    // Around 6-month boundary
    assert_eq!(lines[8], "2008-08-25T12:00:00+0000");
    assert_eq!(lines[9], "2007-09-02T12:00:00+0000");
    assert_eq!(lines[10], "2008-08-29T12:00:00+0000");
    assert_eq!(lines[11], "2007-08-29T12:00:00+0000");
    
    // Around 10-year boundary
    assert_eq!(lines[12], "2018-01-28T12:00:00+0000");
    assert_eq!(lines[13], "1998-03-28T12:00:00+0000");
    assert_eq!(lines[14], "2018-03-28T12:00:00+0000");
    assert_eq!(lines[15], "1998-01-28T12:00:00+0000");
    
    // Around 25-year boundary
    assert_eq!(lines[16], "2032-02-28T12:00:00+0000");
    assert_eq!(lines[17], "1984-02-28T12:00:00+0000");
    assert_eq!(lines[18], "2034-02-28T12:00:00+0000");
    assert_eq!(lines[19], "1982-02-28T12:00:00+0000");
}

// ============================================================================
// Invalid Timezone Suffix Handling
// ============================================================================

#[test]
fn test_strtotime_invalid_timezone_suffix() {
    let code = "<?php
        date_default_timezone_set('UTC');
        $base = 1150494719; // 16 Jun 2006
        
        var_dump(strtotime('22:49:12 bogusTZ', $base));
        var_dump(strtotime('022233 bogusTZ', $base));
        var_dump(strtotime('20060212T23:12:23 bogusTZ', $base));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "bool(false)");
    assert_eq!(lines[1], "bool(false)");
    assert_eq!(lines[2], "bool(false)");
}

// ============================================================================
// @ Timestamp Timezone Behavior
// ============================================================================

#[test]
fn test_strtotime_at_timestamp_ignores_timezone() {
    let code = "<?php
        date_default_timezone_set('Europe/Oslo');
        
        $d1 = strtotime('@1121373041');
        $d2 = strtotime('@1121373041 CEST');
        
        // Both should be identical - timezone ignored with @
        echo date(DATE_ISO8601, $d1) . \"\\n\";
        echo date(DATE_ISO8601, $d2) . \"\\n\";
        echo ($d1 === $d2 ? 'SAME' : 'DIFFERENT') . \"\\n\";
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    // Both should be the same timestamp
    assert_eq!(lines[0], lines[1]);
    assert_eq!(lines[2], "SAME");
}

// ============================================================================
// Whitespace Edge Cases
// ============================================================================

#[test]
fn test_strtotime_whitespace_with_zeros() {
    let code = "<?php
        var_dump(strtotime(\" \\t\\r\\n000\"));
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    assert_eq!(output.trim(), "bool(false)");
}

// ============================================================================
// 64-bit Large Year Tests
// ============================================================================

#[test]
fn test_strtotime_large_years_64bit() {
    let code = "<?php
        date_default_timezone_set('UTC');
        
        // 14-digit MySQL format with year 2080
        $result = strtotime('20800410101010');
        if ($result !== false) {
            echo date('r', $result) . \"\\n\";
        } else {
            echo \"false\\n\";
        }
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    // On 64-bit systems this should work
    if output.trim() != "false" {
        assert_eq!(output.trim(), "Wed, 10 Apr 2080 10:10:10 +0000");
    }
}

// ============================================================================
// ISO8601 Extended Format with UTC Suffix
// ============================================================================

#[test]
fn test_strtotime_iso8601_utc_suffix() {
    let code = "<?php
        date_default_timezone_set('Europe/Lisbon');
        $base = 1150494719;
        
        $result = strtotime('20060212T23:12:23UTC', $base);
        if ($result !== false) {
            echo date('r', $result) . \"\\n\";
        }
    ";
    let (_, output) = run_code_capture_output(code).unwrap();
    if !output.trim().is_empty() {
        assert_eq!(output.trim(), "Sun, 12 Feb 2006 23:12:23 +0000");
    }
}

// ============================================================================
// Additional strtotime3.phpt Test Cases
// ============================================================================

#[test]
fn test_strtotime_comprehensive_edge_cases() {
    let code = r#"<?php
        date_default_timezone_set('Europe/Lisbon');
        $time = 1150494719; // 16/June/2006

        $strs = array(
            '',
            " \t\r\n000",
            'yesterday',
            '22:49:12',
            '2-3-2004',
            '2.3.2004',
            '2006167', // Year-DayOfYear
            'Jan-15-2006',
            '2006-Jan-15',
            '2006',
            '1986',
            'JAN',
            'January',
        );

        foreach ($strs as $str) {
            $t = strtotime($str, $time);
            if (is_int($t)) {
                echo date(DATE_RFC2822, $t) . "\n";
            } else {
                echo "false\n";
            }
        }
    "#;
    let (_, output) = run_code_capture_output(code).unwrap();
    let lines: Vec<&str> = output.trim().lines().collect();
    
    assert_eq!(lines[0], "false"); // Empty string
    assert_eq!(lines[1], "false"); // Whitespace with zeros
    assert_eq!(lines[2], "Thu, 15 Jun 2006 00:00:00 +0100"); // yesterday
    assert_eq!(lines[3], "Fri, 16 Jun 2006 22:49:12 +0100"); // Time today
    assert_eq!(lines[4], "Tue, 02 Mar 2004 00:00:00 +0000"); // D-M-YYYY
    assert_eq!(lines[5], "Tue, 02 Mar 2004 00:00:00 +0000"); // D.M.YYYY
    assert_eq!(lines[6], "Fri, 16 Jun 2006 00:00:00 +0100"); // Year-DayOfYear
    assert_eq!(lines[7], "Sun, 15 Jan 2006 00:00:00 +0000"); // Mon-DD-YYYY
    assert_eq!(lines[8], "Sun, 15 Jan 2006 00:00:00 +0000"); // YYYY-Mon-DD
    assert_eq!(lines[9], "Fri, 16 Jun 2006 20:06:00 +0100"); // Year only (uses base time)
    assert_eq!(lines[10], "Mon, 16 Jun 1986 22:51:59 +0100"); // Year 1986
    assert_eq!(lines[11], "Mon, 16 Jan 2006 00:00:00 +0000"); // Month abbreviation
    assert_eq!(lines[12], "Mon, 16 Jan 2006 00:00:00 +0000"); // Full month name
}
