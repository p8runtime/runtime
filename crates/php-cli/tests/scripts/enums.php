<?php
enum Suit: string { case Hearts = 'H'; case Spades = 'S'; }
enum Pure { case A; case B; }
var_dump(Suit::Hearts, Pure::A); print_r(Suit::Hearts); echo "\n"; print_r(Pure::B); echo "\n"; var_export(Suit::Hearts); echo "\n";
var_dump(Suit::cases());
try { Suit::from('X'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
enum N: int { case One = 1; } try { N::from(5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(Suit::tryFrom('X'), Suit::Hearts instanceof UnitEnum, Suit::Hearts instanceof BackedEnum, Pure::A instanceof BackedEnum);
try { $x = Suit::Hearts; $x->name = 'z'; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { new Pure; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(Suit::Hearts == Suit::Hearts, Suit::Hearts === Suit::Spades, Suit::Hearts < Suit::Spades);
enum Status: int implements JsonSerializable {
    case Active = 1; case Inactive = 0;
    public function label(): string { return match($this) { self::Active => 'on', self::Inactive => 'off' }; }
    public static function fromLabel(string $l): self { foreach (self::cases() as $c) if ($c->label() === $l) return $c; throw new Exception("none"); }
    public function jsonSerialize(): mixed { return $this->label(); }
}
echo Status::Active->label(), Status::fromLabel('off')->name, Status::from('1')->name, "\n";
echo Status::Inactive->value, " ", count(Status::cases()), "\n";
$fn = fn(Status $s) => $s === Status::Active ? 'yes' : 'no'; echo $fn(Status::Active), $fn(Status::Inactive), "\n";
var_dump(Status::Active instanceof Status, Status::tryFrom(9));
var_export([Pure::A, 'k' => Suit::Spades]); echo "\n";
$arr = [Pure::A, Pure::B]; var_dump(in_array(Pure::B, $arr, true), array_search(Pure::B, $arr, true));
echo gettype(Pure::A), " ", get_class(Suit::Hearts), "\n";
