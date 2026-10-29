<?php
// mail() hands sendmail_path the message as PHP writes it: To and Subject
// made single lines, the extra headers (string or array), a blank line,
// the message. Refused headers send nothing.
var_dump(mail("ada@example.com \r\n", "Hi\tthere\r\n folded", "Line 1\nLine 2", "From: p8 <p8@example.com>\r\nX-Test: 1\n\n"));
var_dump(mail('b@example.com', 'Arr', 'Body', ['From' => 'x@example.com', 'X-List' => ['a', 'b']]));
var_dump(mail('c@example.com', 'Bad', 'Body', "\r\nFrom: x"));
foreach ([['To' => 'x'], [1 => 'x'], ['From' => ['a']], ['X-A' => "a\nb"], ['Bad Name' => 'v'], ['X-N' => 5]] as $h) {
    try {
        mail('d@example.com', 's', 'm', $h);
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
var_dump(mail('e@example.com', 'Params', 'Body', '', '-fsender@example.com'));
