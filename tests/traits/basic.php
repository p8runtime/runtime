<?php
trait Logger {
    public function log(string $msg): string {
        return "[" . get_class($this) . "] " . $msg;
    }
}

trait Formatter {
    public function format(string $msg): string {
        return strtoupper($msg);
    }
}

class Worker {
    use Logger, Formatter;

    public function work(): string {
        return $this->log($this->format("starting"));
    }
}

$worker = new Worker();
echo $worker->work() . "\n";
