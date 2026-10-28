// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The holes in a code clip: an SVG with `{{ expression }}` in it, filled
//! once a frame with the time, so a number counts up, a bar grows or a
//! route draws on. See svg.rs for how the frames become a clip.
//!
//! The expressions are arithmetic over a handful of names, small enough
//! to read at a glance and nothing that can reach outside the file:
//!
//! - `t` the seconds since the clip began, `d` its length, `p` = `t / d`,
//!   `frame` the frame's number and `fps` the frames a second, `pi`.
//! - `+ - * / %`, comparisons (`<`, `<=`, `>`, `>=`, `==`, `!=`, true is
//!   1), `cond ? a : b` and brackets.
//! - `ease(x)`, `ease_in(x)`, `ease_out(x)`, `pop(x)` (overshoots, then
//!   settles), `between(t, a, b)` (0 before `a`, 1 after `b`, a straight
//!   line between), `clamp(x, lo, hi)`, `lerp(a, b, x)`, `min`, `max`,
//!   `abs`, `sqrt`, `pow`, `sin`, `cos`, `floor`, `ceil`, `round(x)` or
//!   `round(x, places)`, and `num(x)` or `num(x, places)`: the number with
//!   its thousands grouped, "12,500".

/// A code clip's source, split into its words and its holes.
#[derive(Clone, Debug)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Clone, Debug)]
enum Part {
    Text(String),
    Hole(Expr),
}

/// The names a hole can read, for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    /// Seconds since the clip began.
    pub t: f64,
    /// The clip's length in seconds.
    pub d: f64,
    /// The frame's number, from 0.
    pub frame: u64,
    /// Frames a second.
    pub fps: f64,
}

/// Whether `source` has holes to fill: a code clip, not a still SVG.
pub fn is_animated(source: &str) -> bool {
    source.contains("{{")
}

impl Template {
    /// Splits `source` at its holes and reads each. A hole that does not
    /// read is an error naming its line, for whoever wrote it to fix.
    pub fn parse(source: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut rest = source;
        let mut consumed = 0;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                parts.push(Part::Text(rest[..open].to_owned()));
            }
            let line = source[..consumed + open].lines().count().max(1);
            let after = &rest[open + 2..];
            let close = after
                .find("}}")
                .ok_or_else(|| format!("line {line}: a {{{{ with no }}}} after it"))?;
            let expression = &after[..close];
            let expr = Parser::new(expression)
                .whole()
                .map_err(|why| format!("line {line}: {{{{{}}}}}: {why}", expression.trim()))?;
            parts.push(Part::Hole(expr));
            let used = open + 2 + close + 2;
            consumed += used;
            rest = &rest[used..];
        }
        if !rest.is_empty() {
            parts.push(Part::Text(rest.to_owned()));
        }
        Ok(Template { parts })
    }

    /// The source with every hole filled for `moment`.
    pub fn fill(&self, moment: Moment) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Hole(expr) => match expr.eval(&moment) {
                    Value::Number(number) => out.push_str(&number_text(number)),
                    Value::Text(text) => out.push_str(&text),
                },
            }
        }
        out
    }
}

/// A number as SVG wants it: no exponent, at most three places, no
/// trailing zeros.
fn number_text(number: f64) -> String {
    if !number.is_finite() {
        return "0".to_owned();
    }
    let text = format!("{:.3}", number);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// `number` rounded to `places`, its thousands grouped: "12,500.5".
fn grouped(number: f64, places: usize) -> String {
    let text = format!("{:.*}", places, number.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut digits = String::new();
    for (at, digit) in whole.chars().enumerate() {
        if at > 0 && (whole.len() - at) % 3 == 0 {
            digits.push(',');
        }
        digits.push(digit);
    }
    let sign = if number < 0.0 && text.chars().any(|c| ('1'..='9').contains(&c)) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{digits}")
    } else {
        format!("{sign}{digits}.{fraction}")
    }
}

#[derive(Clone, Debug)]
enum Value {
    Number(f64),
    Text(String),
}

impl Value {
    fn number(&self) -> f64 {
        match self {
            Value::Number(number) => *number,
            Value::Text(text) => text.replace(',', "").parse().unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    Number(f64),
    Name(Name),
    Negate(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Choose(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}

#[derive(Clone, Copy, Debug)]
enum Name {
    T,
    D,
    P,
    Frame,
    Fps,
    Pi,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Less,
    LessEq,
    More,
    MoreEq,
    Equal,
    NotEqual,
}

#[derive(Clone, Copy, Debug)]
enum Function {
    Ease,
    EaseIn,
    EaseOut,
    Pop,
    Between,
    Clamp,
    Lerp,
    Min,
    Max,
    Abs,
    Sqrt,
    Pow,
    Sin,
    Cos,
    Floor,
    Ceil,
    Round,
    Num,
}

impl Function {
    fn named(name: &str) -> Option<(Function, usize, usize)> {
        // The function, and the fewest and most arguments it takes.
        Some(match name {
            "ease" => (Function::Ease, 1, 1),
            "ease_in" => (Function::EaseIn, 1, 1),
            "ease_out" => (Function::EaseOut, 1, 1),
            "pop" => (Function::Pop, 1, 1),
            "between" => (Function::Between, 3, 3),
            "clamp" => (Function::Clamp, 3, 3),
            "lerp" => (Function::Lerp, 3, 3),
            "min" => (Function::Min, 2, 2),
            "max" => (Function::Max, 2, 2),
            "abs" => (Function::Abs, 1, 1),
            "sqrt" => (Function::Sqrt, 1, 1),
            "pow" => (Function::Pow, 2, 2),
            "sin" => (Function::Sin, 1, 1),
            "cos" => (Function::Cos, 1, 1),
            "floor" => (Function::Floor, 1, 1),
            "ceil" => (Function::Ceil, 1, 1),
            "round" => (Function::Round, 1, 2),
            "num" => (Function::Num, 1, 2),
            _ => return None,
        })
    }
}

impl Expr {
    fn eval(&self, moment: &Moment) -> Value {
        let n = |expr: &Expr| expr.eval(moment).number();
        let unit = |x: f64| x.clamp(0.0, 1.0);
        Value::Number(match self {
            Expr::Number(number) => *number,
            Expr::Name(name) => match name {
                Name::T => moment.t,
                Name::D => moment.d,
                Name::P => {
                    if moment.d > 0.0 {
                        unit(moment.t / moment.d)
                    } else {
                        1.0
                    }
                }
                Name::Frame => moment.frame as f64,
                Name::Fps => moment.fps,
                Name::Pi => std::f64::consts::PI,
            },
            Expr::Negate(inner) => -n(inner),
            Expr::Binary(op, a, b) => {
                let (a, b) = (n(a), n(b));
                let truth = |yes: bool| if yes { 1.0 } else { 0.0 };
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div if b == 0.0 => 0.0,
                    Op::Div => a / b,
                    Op::Rem if b == 0.0 => 0.0,
                    Op::Rem => a % b,
                    Op::Less => truth(a < b),
                    Op::LessEq => truth(a <= b),
                    Op::More => truth(a > b),
                    Op::MoreEq => truth(a >= b),
                    Op::Equal => truth(a == b),
                    Op::NotEqual => truth(a != b),
                }
            }
            Expr::Choose(cond, yes, no) => {
                return if n(cond) != 0.0 {
                    yes.eval(moment)
                } else {
                    no.eval(moment)
                };
            }
            Expr::Call(function, args) => {
                let arg = |at: usize| args.get(at).map(n).unwrap_or(0.0);
                let x = arg(0);
                match function {
                    Function::Ease => {
                        let x = unit(x);
                        if x < 0.5 {
                            4.0 * x * x * x
                        } else {
                            1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
                        }
                    }
                    Function::EaseIn => unit(x).powi(3),
                    Function::EaseOut => 1.0 - (1.0 - unit(x)).powi(3),
                    Function::Pop => {
                        // easeOutBack: past 1 by a tenth, then back.
                        let (c1, x) = (1.70158, unit(x));
                        let c3 = c1 + 1.0;
                        1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
                    }
                    Function::Between => {
                        let (a, b) = (arg(1), arg(2));
                        if b <= a {
                            if x >= b { 1.0 } else { 0.0 }
                        } else {
                            unit((x - a) / (b - a))
                        }
                    }
                    Function::Clamp => x.clamp(arg(1).min(arg(2)), arg(2).max(arg(1))),
                    Function::Lerp => x + (arg(1) - x) * arg(2),
                    Function::Min => x.min(arg(1)),
                    Function::Max => x.max(arg(1)),
                    Function::Abs => x.abs(),
                    Function::Sqrt => x.max(0.0).sqrt(),
                    Function::Pow => x.powf(arg(1)),
                    Function::Sin => x.sin(),
                    Function::Cos => x.cos(),
                    Function::Floor => x.floor(),
                    Function::Ceil => x.ceil(),
                    Function::Round => {
                        let scale = 10f64.powi(arg(1).clamp(0.0, 6.0) as i32);
                        (x * scale).round() / scale
                    }
                    Function::Num => {
                        return Value::Text(grouped(x, arg(1).clamp(0.0, 6.0) as usize));
                    }
                }
            }
        })
    }
}

/// A recursive-descent reader, lowest precedence first: choice,
/// comparison, sum, product, sign, call or atom.
struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser { text, at: 0 }
    }

    fn whole(mut self) -> Result<Expr, String> {
        let expr = self.choice()?;
        self.skip();
        match self.rest().chars().next() {
            None => Ok(expr),
            Some(c) => Err(format!("did not expect '{c}'")),
        }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn skip(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip();
        if self.rest().starts_with(token) {
            self.at += token.len();
            true
        } else {
            false
        }
    }

    fn choice(&mut self) -> Result<Expr, String> {
        let cond = self.comparison()?;
        if !self.eat("?") {
            return Ok(cond);
        }
        let yes = self.choice()?;
        if !self.eat(":") {
            return Err("a ? with no : after it".to_owned());
        }
        let no = self.choice()?;
        Ok(Expr::Choose(Box::new(cond), Box::new(yes), Box::new(no)))
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.sum()?;
        loop {
            let op = if self.eat("<=") {
                Op::LessEq
            } else if self.eat(">=") {
                Op::MoreEq
            } else if self.eat("==") {
                Op::Equal
            } else if self.eat("!=") {
                Op::NotEqual
            } else if self.eat("<") {
                Op::Less
            } else if self.eat(">") {
                Op::More
            } else {
                return Ok(left);
            };
            left = Expr::Binary(op, Box::new(left), Box::new(self.sum()?));
        }
    }

    fn sum(&mut self) -> Result<Expr, String> {
        let mut left = self.product()?;
        loop {
            let op = if self.eat("+") {
                Op::Add
            } else if self.eat("-") {
                Op::Sub
            } else {
                return Ok(left);
            };
            left = Expr::Binary(op, Box::new(left), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Expr, String> {
        let mut left = self.sign()?;
        loop {
            let op = if self.eat("*") {
                Op::Mul
            } else if self.eat("/") {
                Op::Div
            } else if self.eat("%") {
                Op::Rem
            } else {
                return Ok(left);
            };
            left = Expr::Binary(op, Box::new(left), Box::new(self.sign()?));
        }
    }

    fn sign(&mut self) -> Result<Expr, String> {
        if self.eat("-") {
            return Ok(Expr::Negate(Box::new(self.sign()?)));
        }
        if self.eat("+") {
            return self.sign();
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, String> {
        self.skip();
        if self.eat("(") {
            let inner = self.choice()?;
            if !self.eat(")") {
                return Err("a ( with no ) after it".to_owned());
            }
            return Ok(inner);
        }
        let rest = self.rest();
        let Some(first) = rest.chars().next() else {
            return Err("an expression ends too soon".to_owned());
        };
        if first.is_ascii_digit() || first == '.' {
            let len = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            let number = rest[..len]
                .parse()
                .map_err(|_| format!("'{}' is not a number", &rest[..len]))?;
            self.at += len;
            return Ok(Expr::Number(number));
        }
        if first.is_ascii_alphabetic() || first == '_' {
            let len = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let word = &rest[..len];
            self.at += len;
            if self.eat("(") {
                let (function, fewest, most) =
                    Function::named(word).ok_or_else(|| format!("no function '{word}'"))?;
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.choice()?);
                        if self.eat(")") {
                            break;
                        }
                        if !self.eat(",") {
                            return Err(format!("{word}( with no ) after it"));
                        }
                    }
                }
                if args.len() < fewest || args.len() > most {
                    return Err(format!("{word} takes {fewest} to {most} values"));
                }
                return Ok(Expr::Call(function, args));
            }
            let name = match word {
                "t" => Name::T,
                "d" => Name::D,
                "p" => Name::P,
                "frame" => Name::Frame,
                "fps" => Name::Fps,
                "pi" => Name::Pi,
                _ => return Err(format!("no name '{word}' (t, d, p, frame, fps, pi)")),
            };
            return Ok(Expr::Name(name));
        }
        Err(format!("did not expect '{first}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t: f64) -> Moment {
        Moment {
            t,
            d: 2.0,
            frame: (t * 30.0) as u64,
            fps: 30.0,
        }
    }

    fn fill(source: &str, t: f64) -> String {
        Template::parse(source).expect("reads").fill(at(t))
    }

    #[test]
    fn holes_fill_with_the_moment() {
        assert_eq!(
            fill("<rect width=\"{{ p * 400 }}\"/>", 1.0),
            "<rect width=\"200\"/>"
        );
        assert_eq!(fill("{{ 1 + 2 * 3 }} {{ (1 + 2) * 3 }}", 0.0), "7 9");
        assert_eq!(fill("{{ t > 1 ? 1 : 0.5 }}", 1.5), "1");
        assert_eq!(fill("{{ -t }}", 0.0), "0");
        assert_eq!(fill("{{ 10 / 3 }}", 0.0), "3.333");
        assert_eq!(fill("no holes", 0.0), "no holes");
    }

    #[test]
    fn the_functions_shape_time() {
        assert_eq!(
            fill("{{ ease(0) }} {{ ease(0.5) }} {{ ease(1) }}", 0.0),
            "0 0.5 1"
        );
        assert_eq!(fill("{{ between(t, 1, 2) }}", 0.5), "0");
        assert_eq!(fill("{{ between(t, 1, 2) }}", 1.5), "0.5");
        assert_eq!(fill("{{ between(t, 1, 2) }}", 3.0), "1");
        assert_eq!(fill("{{ pop(1) }}", 0.0), "1");
        assert!(
            fill("{{ pop(0.7) }}", 0.0)
                .parse::<f64>()
                .expect("a number")
                > 1.0
        );
        assert_eq!(
            fill("{{ round(2.345, 2) }} {{ lerp(10, 20, 0.5) }}", 0.0),
            "2.35 15"
        );
    }

    #[test]
    fn num_groups_the_thousands() {
        assert_eq!(fill("{{ num(12500) }}", 0.0), "12,500");
        assert_eq!(fill("{{ num(p * 1000000) }}", 1.0), "500,000");
        assert_eq!(fill("{{ num(1234.567, 1) }}", 0.0), "1,234.6");
        assert_eq!(fill("{{ num(-950) }}", 0.0), "-950");
        assert_eq!(fill("{{ num(0.2) }}", 0.0), "0");
    }

    #[test]
    fn a_bad_hole_says_where_and_why() {
        let error = Template::parse("<svg>\n<rect width=\"{{ q * 2 }}\"/>").expect_err("q");
        assert!(error.starts_with("line 2:"), "{error}");
        assert!(error.contains("no name 'q'"), "{error}");
        assert!(
            Template::parse("{{ wobble(t) }}")
                .expect_err("f")
                .contains("no function")
        );
        assert!(Template::parse("{{ t + }}").is_err());
        assert!(Template::parse("{{ t").expect_err("open").contains("no }}"));
        assert!(
            Template::parse("{{ clamp(t) }}")
                .expect_err("args")
                .contains("takes")
        );
    }
}
