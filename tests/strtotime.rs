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
