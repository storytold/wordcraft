# Calculations

WordCraft can calculate inside a document the way Mathcad does: you write an equation with
units, and the result is worked out and shown beside it. Change an input and every result that
depends on it, anywhere in the document, updates. Insert one from Insert › Calculation
(Ctrl+Alt+=, ⌘⌥= on a Mac). To change one, double-click it, press F2 with the caret beside it, or
select it and press Enter.

Calculations work in the main text and its tables. A calculation pasted or moved anywhere else (a
text box, header, footer, footnote or comment) stops being one: it becomes a plain equation
showing its calculation with no result, marked ⚠ "not calculated here". A quoted value there shows
`?`.

## Notation

| You write | It means |
| --- | --- |
| `P := 1500 lbf` | defines `P` (`≔` is accepted as well) |
| `P/A_b =` | works out the expression and shows the result after the `=` |
| `σ := P/A_b =` | both: defines `σ` and shows it |
| `P/A_b = ? ksi` or `P/A_b = ksi` | shows the result in the unit you pick |
| `f(x, y) := x^2 + y` | defines a function of your own |
| `2 P` | implicit multiplication |
| `x²` | the same as `x^2` |

Names are global. A name is defined once, anywhere in the document, and can be used above or
below its definition. Defining a name twice is an error ("already defined on page N — choose a
unique name"), and so is a circular definition (`a := b`, `b := a`).

A name can't be a unit, a built-in function or a constant. That rules out `m`, `A`, `L`, `g`,
`s`, `N`, `W`, `J`, `kN`, `min`, `bar`, `day`, `ton`, `cal`, `sin`, `erf`, `π`, `e`, `g_0`, `c_0`,
`R_u` and the rest (`h`, `lb` and `R` are not units, so they are free). Use a subscript instead:
`A_b`, `L_1`. A subscript can't be empty (`a__b` and `x_` are refused). The array
functions (`sum`, `length`, `lookup` and the rest; see
[Vectors, matrices and data tables](#vectors-matrices-and-data-tables)) are reserved too, and
so are `Given`, `Find`, `Minerr` and `root` (see [Solving](#solving)). A
definition that uses one of these names shows the reserved-name error, and every calculation that
uses the name shows "depends on a calculation with an error" until the definition is renamed: the
name never quietly means the unit, function or constant instead.

## Constants

| Name | Value |
|---|---|
| `π` or `pi` | 3.141592653589793… |
| `e` | 2.718281828459045… |
| `g` | standard gravity, 9.80665 m/s², as in Mathcad (see below) |
| `g_0` | standard gravity, under either setting |
| `c_0` | speed of light, 299 792 458 m/s |
| `R_u` | molar gas constant, 8.314462618 J/(mol·K) |

The constants carry their units, so `W_t := 5 kg·g = ? N` shows 49.033 N and `1 lbm·g = ? lbf`
shows 1 lbf.

### `g`: gravity or gram

By default `g` means standard gravity, as in Mathcad, and the gram is written `gm` (`250 gm`).
`kg`, `mg` and the other prefixed grams are unchanged. To use the SI convention instead, where `g`
is the gram and gravity is `g_0`, choose **Gram (SI)** under "g means" in References ›
Calculation Options. The setting is saved in the document, `.docx` included, so a report
calculates the same on every computer. Changing it recalculates the document, so check any
calculation that uses `g`. While `g` means gravity, a display unit written with `g` (`= ? g`) is
refused; write `gm`.

## Symbols

In the calculation box, **Insert Symbol** lists Greek letters, operators (`≔ · × ± √ ² ³ ≤ ≥ ≠ ∑
∧ ∨ ¬`), the temperature units, `Å` and `%`, and the constants; picking one puts it at the cursor.
You can also type a backslash name followed by a space, as in Word: `\sigma` → σ, `\theta` → θ, `\Delta` → Δ,
`\mu` → μ, `\omega` → ω, `\sqrt` → √, `\cdot` → ·, `\times` → ×, `\pm` → ±, `\deg` → °. Any
Greek letter's name works, with a capital for the capital letter. For conditions and sums: `\le` →
≤, `\ge` → ≥, `\ne` → ≠, `\sum` → ∑, `\wedge` → ∧, `\vee` → ∨, `\neg` → ¬.

## Units

SI units with prefixes and US customary units both work: `in`, `ft`, `lbf`, `kip`, `psi`, `ksi`,
`lbm`, `slug`, `Btu`, `hp`, `gal` and so on. Results are checked for dimensions, so adding a length
to a force is an error rather than a number.

- `°C` and `°F` can only be the unit of a whole value. Temperatures on an offset scale can't be
  added together; use `Δ°F` for a difference.
- A result is shown in `°C` or `°F` only when it is an absolute temperature: a `°C`/`°F` value, a
  `K`/`°R` value, or one of those plus or minus a difference (`T_1 + 5 Δ°C`). A difference
  (`T_2 - T_1`, `20 Δ°C`) or anything else made from temperatures (`2 T`, `(T_1 + T_2)/2`) shown
  in `°C` is an error: "°C shows an absolute temperature; use Δ°C for a difference". Show it in
  `Δ°C`, `Δ°F` or `K` instead. For an average temperature write `T_1 + (T_2 - T_1)/2`.
- `J/mol K` is refused as ambiguous. Write `J/(mol·K)`.
- Thousands separators written as spaces (`1 500`) are refused, because a space is multiplication.
  Write `1500`.

### Spellings

Some units have two accepted spellings. Type whichever you like; they mean the same.

| Shown in results | Also accepted |
| --- | --- |
| `hr` | none (see below) |
| `sec` | `s` |
| `Btu` | `BTU` |
| `liter` | `L` |
| `watt` | `W` |
| `joule` | `J` |
| `mol` | `mole` |
| `Å` | `Angstrom` |

When WordCraft picks the unit text itself (a result in its natural units, or one built from base
units such as `m/sec²`), it uses the left column, in SI and US documents alike. Prefixed units keep
their symbols: `ms`, `kW`, `kJ`, `mL`. A unit you wrote yourself shows as you wrote it: `= ? BTU/hr`
shows `BTU/hr`, and `= ? s` shows `s`.

- **The hour is `hr`.** `h` is not a unit, so it is free to use as a name. A document that still
  uses `h` for hours shows "h isn't defined — for hours, use hr" on each calculation that uses it,
  and "unknown unit `h` — for hours, use hr" where `h` is a unit to show a result in
  (`= ? kW·h`) or in a data table's heading (`Time (h)`).
  A time such as `3 hr` shows in `sec` unless you ask for another unit: `= ? hr`.
- **`lb` and `R` are not units.** `lb` reads as force to many engineers, and `R` is a common name.
  Write `lbm` for pound-mass (or `lbf` for pound-force) and `°R` for degrees Rankine. Both words
  are free to use as names.

### Units added for Mathcad

Each factor is Mathcad's.

| Unit | Meaning |
| --- | --- |
| `atm` | standard atmosphere, 101 325 Pa |
| `bar` | 10⁵ Pa; takes no prefixes, so names such as `ubar` stay free |
| `mbar` | millibar, 100 Pa |
| `kbar` | kilobar, 10⁸ Pa |
| `torr` | 101 325/760 Pa |
| `gpm` | `gal/min` |
| `cfm` | `ft³/min` |
| `mph` | `mi/hr` |
| `poise` | 0.1 Pa·sec |
| `cP` | centipoise, 0.01 `poise` |
| `%` | 0.01; written after the number: `5 %` |
| `kgf` | kilogram-force, 9.80665 N |
| `ton` | short ton, 2000 `lbm` |
| `tonne` | metric ton, 1000 kg |
| `cal` | 4.1868 `joule`; takes prefixes (`kcal`) |
| `day` | 86 400 `sec` |
| `yr` | Mathcad's year, 365.2422 `day` |
| `Å` | angstrom, 10⁻¹⁰ m |
| `plf` | pounds per linear foot, `lbf/ft` |
| `pcf` | pounds per cubic foot, `lbf/ft³`: a force per volume, not a mass |
| `ksf` | kips per square foot, `kip/ft²` |

`%` is an ordinary unit: `5 % · 200 =` gives 10, and `x = ? %` shows 0.25 as `25 %`. `Å` is in
the Insert Symbol list; `Angstrom` types the same unit.

Every one of these names is now a unit, so it can't be a variable name. A document that already
defined `bar`, `day`, `ton` or `cal` shows the reserved-name error ("bar is a unit, function or
constant name — choose another name") until the variable is renamed, for instance `bar_1`. Every
calculation that uses the name shows "depends on a calculation with an error" until then, rather
than quietly using the unit.

## How results are shown

- Three decimal places, trailing zeros trimmed.
- Exponential form (`6×10^6`) when the value is 1 000 000 or more, or too small to show at least
  two significant digits at the chosen precision: at three places that is below 0.01, so 0.0005
  shows as `5×10^-4`, not `0.001`. (The threshold is `10^(1 − places)`, never above 1 and never
  below 0.0001.)
- The default unit system is US customary. Choose SI in References › Calculation Options.
- `= ? unit` overrides the unit for one result.
- To number a calculation, right-click it and choose Add Equation Number (Remove Equation Number
  takes it off). The number stays as the result changes and is saved to Word.
- A calculation on an empty line is a display equation, on its own line. Right-click it to choose
  Align Left, Align Center or Align Right.
- To write notes after a calculation on the same line, right-click it and choose Change to Inline,
  then type after it (for example a Tab and "bearing stress, see ref. 3"). The line follows the
  paragraph's alignment and tab stops. A calculation inserted into a line that already has text is
  inline from the start. An inline calculation can't be numbered; Change to Display puts it back on
  its own line.

## Quoted values

Insert › Calculated Value puts a variable's value into a sentence ("each bolt carries 45.271 ksi").
It shows only the body of the result (no name, no `=`) and follows the variable, with an exponent
written in superscript (`2.9×10⁷ psi`). It shows `?` while the value isn't available (an error, or
a name that's gone). A unit the variable can't be shown in is refused when the value is inserted.

## Recalculation

Results update automatically. In manual mode (References › Calculation Options) they are marked
`≈` when an input changes, until you choose References › Recalculate. Typing ordinary text marks
nothing out of date.

A document whose calculations together need too much work (deeply recursive functions, say) is
calculated up to a fixed limit; the calculations past it show the error "too complex to
calculate" rather than a result.

## Word

Saving as `.docx` writes each calculation as an ordinary Word equation, so Word shows the results,
plus hidden calculation source. Opening that file in WordCraft makes the calculations live again.
Quoted values are written the same way.

If an equation is edited in Word, WordCraft notices when you open the file: the status bar says how
many were edited, and each one keeps Word's version with an amber ⚠ after it. Until you resolve it,
it defines nothing: every calculation that uses its name shows the error "depends on a calculation
edited in Word — resolve it" instead of a result, and quoted values of them show `?`. Hover over it
(or put the caret beside it) for two choices: "Restore from calculation" brings back what the
calculation says, and "Keep Word's equation" keeps what was typed in Word and stops it being a
calculation. The ⚠ is WordCraft's: it isn't saved into the Word equation. Calculations inside
tracked deletions are kept when you save and reopen.

## Vectors, matrices and data tables

A calculation can hold a list of values, so one formula gives a result for every case at once:
every pipe size in a design, say. Short lists can be typed; longer ones belong in a table in the
document.

### Typing vectors and matrices

| You write | It means |
| --- | --- |
| `D := [0.840, 1.050, 1.315] in` | a vector of 3 values, all in inches |
| `L_1 := [2 in, 50 mm]` | each element with its own unit |
| `M := [1, 2; 3, 4]` | a 2×2 matrix; `;` starts a new row |

A list in brackets is always a column: `[1, 2, 3]` has 3 rows and 1 column, as in Mathcad. For a
row, transpose it: `transpose([1, 2, 3])` has 1 row and 3 columns. A unit after the bracket
applies to every element. An element can carry its own unit instead, and mixed units are fine
where they make sense (`[2 in, 50 mm]` is two lengths).

### Arithmetic is element by element

`+ − · / ^` work element by element between vectors or matrices of the same shape, and between an
array and a single value. So `t_min := P·D_o/(2·(S·E + P·Y))` gives one `t_min` for each `D_o`.

- Different shapes are an error naming both: "shapes don't match: 3 rows × 1 vs 4 rows × 1".
- The unit rules apply to each element. One element in error makes the whole result an error that
  names the element ("element 1: …").
- Built-in functions such as `sqrt`, `sin`, `ln`, `abs` and `round` work on each element, and so do
  your own functions.
- `|v|` is the absolute value of each element, like `abs(v)`. It is not the vector's magnitude:
  for that, write `sqrt(dot(v, v))`.

**From Mathcad:** in Mathcad, vector × vector is the dot product unless you vectorize it. In
WordCraft it is always element by element, so there is no vectorize operator. A pasted formula
that relied on the dot product must use `dot(a, b)`; a matrix product is `mmul(A, B)`.

### Indexing

`D[2]` is element 2 of `D`, and `M[1, 0]` is row 1, column 0 of `M`; both are drawn as subscripts.
The `[` must touch the name: with anything else before it, `[` starts a list, so `2[1, 2]` is 2
times the list.

Elements count from 0, as in Mathcad, so `D[0]` is the first. To count from 1, choose 1 under
"Arrays count from" in References › Calculation Options; the setting is saved in the document.
Changing this renumbers every index, including quoted elements: after a change from 0 to 1, a
value quoted as element 1 shows the first element, not the second. An index outside the array is
an error that gives the valid range ("index 3 is outside D (0 to 2)").

### Ranges and `t[i] :=`

| You write | It means |
| --- | --- |
| `i := 0..5` | 0, 1, 2, 3, 4, 5 (both ends included) |
| `x := 1, 1.5..3` | 1, 1.5, 2, 2.5, 3 (first, second..last) |
| `j := 0..length(D)-1` | the ends can be any expression |
| `L_x := 0 in, 0.5 in..2 in` | a range with units needs a step |
| `t[j] := 2·D[j]` | defines all of `t` at once, one element for each `j` |

A range is a vector. It can only be the whole right side of a definition: `x := 0..5 =` shows it,
but `0..5 =` alone is an error. A function, or an indexed name like `t[i]`, can't be defined as a
range.

A step that lands on the last value ends exactly on it, however large the values are
(`100000, 100000.1..100002` has 21 elements, ending at 100002). A step too small next to its
values to tell where the range ends, such as 0.1 on values near 10^15, is an error rather than a
guess.

`t[i] := …` is Mathcad's range-and-index pattern, and it is still a single definition of `t`.
Usually the element-by-element form is simpler: `t := 2·D` gives the same vector.

### Functions

| Function | Gives |
| --- | --- |
| `length(v)` | the number of elements |
| `rows(M)`, `cols(M)` | the number of rows or columns |
| `min(…)`, `max(…)` | the smallest or largest element of all the arguments together |
| `sum(v)`, `mean(v)` | the total, or the average, of the elements |
| `dot(a, b)` | the dot product of two vectors of the same length |
| `mmul(A, B)` | the matrix product; A's columns must match B's rows |
| `transpose(M)` | rows and columns swapped |
| `submatrix(A, ir, jr, ic, jc)` | rows `ir` to `jr` and columns `ic` to `jc` of `A`, both ends included, counted from the document's index base |
| `augment(A, B, …)` | the matrices side by side; they must have the same number of rows |
| `stack(A, B, …)` | the matrices one below the other; they must have the same number of columns |
| `lookup(xs, ys, x)` | the `ys` element whose `xs` element equals `x` |
| `linterp(xs, ys, x)` | the straight-line interpolation of `ys` at `x` |
| `spline(xs, ys, x)` | the interpolation along a natural cubic spline through every point |

- `lookup` never guesses. `x` must equal one `xs` element to within rounding, after converting
  units (so `38.1 mm` finds `1.5 in`); no match, or more than one, is an error.
- For `linterp` and `spline`, `xs` must be a single row or column, strictly increasing. Neither
  extrapolates: an `x` outside `xs` is an error, such as "1.6 in is outside NPS (0.5 in to
  1.5 in)".
- `x` can be a vector in `lookup`, `linterp`, `spline` and `interp` (see
  [Mathcad functions](#mathcad-functions)), giving one result for each element.

These names are reserved, like the units and the other functions: `sum`, `length`, `rows`, `cols`,
`mean`, `dot`, `mmul`, `transpose`, `submatrix`, `augment`, `stack`, `lookup`, `linterp` and
`spline` can't be variable names.

### Mathcad functions

These work as they do in Mathcad, with the same spelling and the same numbers. The matrix
functions `submatrix`, `augment` and `stack` are in the table above.

| Function | Gives |
| --- | --- |
| `Ceil(x, y)`, `Floor(x, y)`, `Round(x, y)` | `x` rounded up, down or to the nearest multiple of `y`. `x` and `y` must have the same dimension and `y` must be above 0; the result is in `x`'s unit. `Ceil(2.06 in, 0.125 in)` is `2.125 in` |
| `cspline(vx, vy)`, `lspline(vx, vy)`, `pspline(vx, vy)` | the coefficient vector of the spline through every point, with cubic, linear or parabolic ends |
| `regress(vx, vy, k)` | the coefficient vector of the least-squares polynomial of degree `k` |
| `interp(vs, vx, vy, x)` | the value at `x` of the spline or regression `vs`, over the data `vx` and `vy` |
| `slope(vx, vy)`, `intercept(vx, vy)` | the least-squares line; the slope is in `y`/`x` units and the intercept in `y` units |
| `erf(x)` | the error function of a plain number |
| `csc(θ)`, `cot(θ)`, `secant(θ)` | the reciprocal trigonometric functions; `θ` is an angle (`30°`) or a plain number in radians |
| `atanh(x)` | the inverse hyperbolic tangent |
| `J0(x)`, `J1(x)` | Bessel functions of the first kind, orders 0 and 1, of a plain number |

**Rounding.** Lowercase `ceil`, `floor` and `round` are unchanged: they take one value and give a
whole number (`ceil(2.06)` is 3). With a step, the capitalized names round to a multiple of it, so
`Floor(2.06 in, 0.125 in)` is `2 in`, `Round(2.07 in, 0.125 in)` is `2.125 in`, and a negative `x`
rounds the same way (`Floor(-2.06 in, 0.125 in)` is `-2.125 in`). A step of zero or less, or one in
a different dimension (`Ceil(2.06 in, 0.1 lbf)`), is an error. Names are case sensitive, so the two
spellings never mix.

**`secant`.** Mathcad writes the secant `sec(x)`, but `sec` is the unit for seconds here, so write
`secant(x)`. A calculation that calls `sec(x)` says so: "sec isn't a function — for the secant, use
secant(x)".

**Curve fitting.** `cspline`, `lspline` and `pspline` fit a spline through every point of `vy`
against `vx`; `cspline` and `pspline` need at least 3 points and `lspline` 2. `regress(vx, vy, k)`
fits a polynomial of degree `k`, a whole number from 1 to one less than the number of points. Each
gives a coefficient vector, and `interp` evaluates it:

```
vx := [1, 2, 3, 4]
vy := [1, 4, 9, 16]
s := cspline(vx, vy)
interp(s, vx, vy, 2.5) =          6.25
f := regress(vx, vy, 2)
interp(f, vx, vy, 5) =            25
```

The coefficient vector starts with a header of WordCraft's own, then the values:

| Element | Holds |
| --- | --- |
| 0 | the kind: 1 for `lspline`, 2 for `pspline`, 3 for `cspline`, 4 for `regress` |
| 1 | 3 |
| 2 | the degree `k` for `regress`, 0 for a spline |
| 3 onward | for a spline, its second derivatives at each of `vx`; for `regress`, the coefficients `a_0 … a_k`, `a_0` first |

From element 3 on, `regress`'s coefficients match Mathcad's, so a worksheet that indexes into them
gets the same numbers. The header (elements 0 to 2) is WordCraft's, and a spline's second
derivatives from element 3 on are in WordCraft's layout, not yet confirmed against Mathcad's.

`interp` takes the vector as it came from the fitting function and the same `vx` and `vy`. It
refuses a vector that isn't one of these, or that was made over data of another size or units.
A spline doesn't extrapolate, as for `spline`: an `x` outside `vx` is an error ("5 is outside vx
(1 to 4)"). A `regress` is a formula, so its `interp` evaluates anywhere, outside `vx` too. `x` can
be a vector, giving one result for each element. `slope` and `intercept` need `vx` and `vy` of the
same length.

All of the names in these two sections are reserved: `Ceil`, `Floor`, `Round`, `cspline`,
`lspline`, `pspline`, `regress`, `interp`, `slope`, `intercept`, `erf`, `csc`, `cot`, `secant`,
`atanh`, `J0` and `J1` can't be variable names.

### Data tables

Any table in the main text can supply the values. Right-click it and choose **Use as Calculation
Data** (**Stop Using as Calculation Data** turns it back into an ordinary table). Each column then
defines a vector, named by the heading in row 1:

- `Name (unit)`, such as `D_o (in)`, for values in that unit;
- `Name` alone, such as `qty`, for plain numbers.

The headings follow the usual naming rules: a column name is defined once in the document, and it
can't be a unit or a function name. The table can be anywhere in the document, before or after the
calculations that use it.

A cell holds one number, optionally followed by a unit:

| A cell can hold | Value |
| --- | --- |
| `0.840` | a decimal |
| `3/4` | a fraction, 0.75 |
| `1-1/2` or `1 1/2` | a mixed number, 1.5 (the pipe-size convention; never "1 minus a half") |
| `21.3 mm` | a value with its own unit, which must be the same kind as the heading's |

A thousands separator (`1,500`) is refused, and so is a blank cell or any other text. While `g`
means gravity, the gram in a table is written `gm`, as in calculations.

Editing a cell recalculates like any other edit. Rows deleted with tracked changes are skipped.
Splitting a data table keeps both parts as data tables; the second part's first row becomes its
headings, so check it.

#### Example: ASME B31.3 minimum wall thickness

This is the engine's `pipe_wall_sample` test. A table marked as calculation data:

| NPS (in) | D_o (in) | t_nom (in) |
| --- | --- | --- |
| 1/2 | 0.840 | 0.109 |
| 3/4 | 1.050 | 0.113 |
| 1 | 1.315 | 0.133 |

and these calculations:

```
P := 1000 psi
S := 20000 psi
E := 1
Y := 0.4
t_min := P·D_o/(2·(S·E + P·Y)) =     → [0.021; 0.026; 0.032] in
margin := t_nom - t_min =            → [0.088; 0.087; 0.101] in
min(margin) =                        → 0.087 in
```

The margin is called `margin` because `m` is the meter. Changing the 3/4" `t_nom` to `0.120`
makes `min(margin)` 0.088 in.

A second example (the engine's `flange_torque_sample` test, in a document of its own, since `NPS`
can only be defined once):

| NPS (in) | T_flange (ft·lbf) |
| --- | --- |
| 1/2 | 30 |
| 3/4 | 45 |
| 1 | 60 |
| 1-1/2 | 90 |

```
lookup(NPS, T_flange, 1.5 in) = ? ft·lbf     → 90 ft·lbf
linterp(NPS, T_flange, 1.25 in) = ? ft·lbf   → 75 ft·lbf
linterp(NPS, T_flange, 1.6 in) =             → error: 1.6 in is outside NPS (0.5 in to 1.5 in)
```

#### Table errors

Nothing in a data table is ever read as 0 or guessed. A problem is an error for its column, which
names the column and the row of values (counting the first row under the headings as row 1):

- "t_nom row 2 isn't a number" for a blank cell or text;
- a cell whose unit is the wrong kind for its heading;
- a heading that isn't a valid name, or is a unit or function name.

A column name defined twice in the document follows the usual rule: whichever comes second is the
error.

These are errors for the whole table:

- merged cells;
- a heading row (repeated on each page) other than the first row;
- a row without exactly one cell under each heading;
- fewer than two rows, so no row of values (also when every row of values is deleted with
  tracked changes);
- more than 63 columns.

A column of more than 10,000 values is an error for that column. A red ⚠ on the heading cell
shows the table has errors (on the first cell for an error about the whole table); hover over it
to read them. The table's text is never changed. Every calculation that uses a column in error
shows "depends on table data with an error" instead of a result.

### How results and quoted values show

- A vector or matrix result is drawn as a matrix in brackets, each element formatted as usual.
  When every element is the same kind of quantity, the unit is shown once after the bracket;
  otherwise each element shows its own. `= ? unit` applies to every element.
- A long result is cut visibly, never silently: after 50 rows it shows `⋮` and "(N rows)", and
  after 50 columns `⋯` and "(N columns)".
- A quoted vector (Insert › Calculated Value) is a comma list: "0.109, 0.113, 0.133 in". After 50
  values it ends "… (N values)". To quote one element, fill in the **Element** box: `1` for a
  vector, or a row and a column such as `1, 0` for a matrix.

### In Word

Saving as `.docx` writes vector results as ordinary Word equations with matrices, and a data table
as an ordinary Word table that a reviewer can read and edit. Opening the file in WordCraft makes
the table calculation data again. If its values were edited in Word, the new values are used, and
the status bar says "Calculation data was edited in Word in N tables; results were recalculated",
so the change never goes unnoticed.

A typed list longer than 1,000 rows (or wider than 64 columns) doesn't survive the trip through
Word: it comes back marked as edited in Word. Keep long data in a data table.

## Conditions, text and sums

A calculation can compare values, choose between cases, give a word such as "OK" or "NG", and add
up a series of terms. The everyday uses are a code check (a coefficient that depends on
temperature, a pass or fail for each pipe size) and a sum over a bolt pattern.

### Text values

A text is written in double quotes, straight (`"OK"`) or curly (`“OK”`):

```
verdict := "OK" =                    → "OK"
grades := ["A106 B", "A312 TP316"]
```

- A text can be up to 1,000 characters and can't contain a quote mark.
- Text goes where a word belongs: a definition, a vector of texts, the result of a case or of
  `if(…)`, a `lookup` key or result, and `==` or `≠`. Indexing, `length`, `rows`, `cols` and
  `transpose`, `submatrix`, `augment` and `stack` work on a vector of texts as on numbers (a text with
  numbers in one matrix is an error).
- Text can't be used in arithmetic, with a unit, in `<`, `>`, `≤`, `≥`, or in `min`, `max`, `sum`,
  `mean`, `linterp`, `spline`, `cspline`, `lspline`, `pspline`, `regress`, `interp`, `slope` or
  `intercept`. The error says so and names the text (`"OK" is text — it can't be used in
  arithmetic`); a text is never turned into a number.
- Two texts are equal when they match after ignoring spaces at either end and treating a run of
  spaces as one. Capital letters count: `"a" == "A"` is 0.
- A text result shows in quotes (`check = "OK"`), and a vector of texts as a bracketed column. A
  quoted value (Insert › Calculated Value) shows the text without quotes: "the check is OK", and a
  vector as a list, "OK, OK, NG".

### Comparisons

| You type | Shown as | Meaning |
| --- | --- | --- |
| `<` `>` | `<` `>` | less than, greater than |
| `<=` or `≤` | `≤` | at most |
| `>=` or `≥` | `≥` | at least |
| `==` | a bold `=` | equal (so it can't be mistaken for a result) |
| `!=` or `≠` | `≠` | not equal |

- Units are checked first: `1 in ≥ 1 lbf` is an error, not true or false. Values are compared after
  converting units, so `38.1 mm == 1.5 in` is true and `100 °C == 212 °F` is true.
- A comparison gives 1 (true) or 0 (false), and works in arithmetic: `sum(check == "NG")` counts
  the failures, and `2 + (x > 3)` is 3 when `x` is 5. On vectors, a comparison works element by
  element, like arithmetic.
- Values within a relative 1e-12 of each other count as equal for all six operators, so a check
  that lands exactly on its limit doesn't flip because of unit conversion: `38.1 mm > 1.5 in` is
  false and `38.1 mm ≤ 1.5 in` is true. Zero is equal only to exactly zero. To compare with zero,
  use a tolerance: `abs(x) < 1e-9 m`, not `x == 0 m`.
- Comparisons chain: `0 ≤ x < 1` means `0 ≤ x and x < 1`. Like `and`, a chain stops at the first
  link that is false, so `0 < b < 1/b` is 0, not an error, when `b` is 0. On vectors this happens
  per element.
- An absolute temperature (a `°C` or `°F` value, or one plus or minus a difference) compares with
  another absolute temperature or with a `K` or `°R` value: `T ≤ 900 °F` and `T ≤ 755 K` both
  work. It can't be compared with a temperature difference, or with an average written
  `(T_1 + T_2)/2`: that is an error, because the comparison would give a wrong answer that looks
  right. Write an average as `T_1 + (T_2 − T_1)/2`. Two differences compare directly
  (`ΔT ≤ 10 Δ°F`).

### And, or and not

`and`, `or` and `not` (also typed `∧`, `∨`, `¬`, or with `\wedge`, `\vee`, `\neg`) combine
comparisons. They are always shown as `∧`, `∨` and `¬`, however they were typed, and give 1 or 0.
They take dimensionless numbers, where any nonzero value counts as true.
If the interface font can't draw `∧` or `∨`, the calculation box shows the words `and` and `or`
instead; they mean the same.

```
ok := P ≤ P_max and T ≤ T_max =
```

- `not` binds looser than a comparison, so `not x < 3` means `not (x < 3)`, and it is shown as
  `¬(x<3)`. `and` binds tighter than `or`.
- When the left side already decides the result, the right side isn't worked out:
  `b ≠ 0 and 1/b > 1` is 0, not an error, when `b` is 0. On vectors this happens per element.

### Cases

A definition can choose its value from cases, tried in order. The first true condition wins:

```
Y := 0.4 if T ≤ 900 °F, 0.5 if T ≤ 950 °F, 0.7 otherwise =
```

The coefficients here only illustrate the syntax; take the real ones from the code. With
`T := 925 °F` the result is 0.5, and `T := 500 °C` gives 0.5 as well (932 °F). It is shown with a
left brace, one line per case.

- Cases can only be the whole right side of a definition, including a function definition
  (`Y(T) := 0.4 if T ≤ 900 °F, 0.7 otherwise`). They can't be used inside parentheses or in a
  larger formula; use `if(…)` there. Their commas would be confused with list commas.
- `otherwise` is optional, but if no case is true the result is an error, never 0:
  `no case applies (T = 1000 °F)`.
- A condition that starts with a parenthesis needs a space after `if`:
  `1 if (a > b), 2 otherwise`. Written `if(a > b)` with nothing between, it is the `if` function
  instead (below).
- With a vector, each element picks its own case, and an element only fails if the case it picks
  fails: `sqrt(x) if x ≥ 0, 0 otherwise` works for a vector `x` with negative entries, giving the
  square root where `x ≥ 0` and 0 elsewhere.
- Inside a case, whole arrays stay whole. `sum(x)`, `x[0]` and the tables of `lookup(xs, ys, …)`
  see all of `x`, `xs` and `ys`, as they do outside a case; only the choice of result is per
  element.
- The value looked up or interpolated at, the `x` of `lookup`, `linterp`, `spline` and `interp`, is per
  element like arithmetic. With `xs` ending at 20, `linterp(xs, ys, x) if x ≤ 20, 0 otherwise`
  works for a vector `x` with elements past 20, giving 0 there. Likewise an element of `x` with no
  match in `xs` fails only if it picks that case.
- The results can be text: `"OK" if margin ≥ 0 in, "NG" otherwise`. A single value may be text in
  one case and a number in another, but a vector can't mix them: if one element picks a number and
  another picks text, the result is an error that names the element ("element 1 is text but
  element 0 is a number").

### The `if` function

`if(c, a, b)` gives `a` where `c` is true and `b` where it is false. It is for use inside a
larger expression, and it is how a pasted Mathcad `if(…)` works. It works out only the branch it
picks, per element for vectors, so `if(x ≠ 0, 1/x, 0)` doesn't fail on a zero element. It is shown
as a function call.

### Sums

`∑(i = first..last) term` adds `term` for each `i`. Type `∑` with `\sum`.

```
n := 3
a := [1, 2, 3, 4]
b := [5, 6, 7, 8]
∑(i = 1..n) a[i]·b[i] =              → 65
∑(i = 1, 3..9) i =                   → 25 (1 + 3 + 5 + 7 + 9)
```

The range follows the rules of a range definition: `first..last`, or `first, second..last` to
step, with both ends included. The sum is drawn as a real ∑ with the limits below and above,
including in Word.

A bolt-pattern sum, the moment of inertia of the bolt areas about the center:

```
A_b := 0.3 in^2
r := [4 in, 8 in]
I_b := ∑(i = 0..length(r) − 1) A_b·r[i]^2 =      → 24 in^4
```

- The index belongs to that sum alone, so two sums can both use `i`, and sums can be nested
  (`∑(i = 0..1) ∑(j = 0..1) a[i]·b[j]`). It can't be a name already defined in the document, a
  parameter of the function around it, or the index of a sum it is inside; each of those is an
  error asking for another name.
- A sum covers one product term, like `·`: `∑(i = 1..n) a[i]·b[i] + 1` is the sum plus 1. To sum
  more, use parentheses: `∑(i = 1..n) (a[i] + 1)`. The display draws the parentheses, so how it
  was read is visible.
- The terms must be addable under the unit rules. They can be vectors of one shape, which add
  element by element.
- A range whose last value is below its first is an error ("the sum's range is empty — its last
  value is below its first"), never a sum of 0. A huge range is refused ("more than 10,000
  elements") rather than left to hang.

### Text columns in data tables

A heading ending in `(text)` makes a column of texts instead of numbers:

| Material (text) | S_table (ksi) |
| --- | --- |
| A106 B | 20 |
| A312 TP316 | 20 |
| A333 6 | 20 |

```
S_allow := lookup(Material, S_table, "A106 B") = ? ksi      → 20 ksi
```

Without `= ? ksi` the result shows in the document's unit system (20000 psi in US customary).
Every other column stays strictly numeric, so a typo in a number cell is still an error and never
quietly becomes text. In a text column:

- a cell's value is its text with the spaces at either end removed, and a number is text (`"6"`);
- a cell can't be empty (an error, as in a number column) or hold a quote mark of any kind: `"`,
  `“` or `”`, so a smart quote counts;
- `lookup` by text matches as `==` does, ignoring spaces at the ends and runs of spaces but not
  capitals, so `"a106 b"` doesn't find `A106 B`. A text that isn't in the column, or that matches
  more than one cell, is an error.

### Example: a check for each pipe size

Add these to the ASME B31.3 example under [Data tables](#data-tables):

```
check := "OK" if margin ≥ 0 in, "NG" otherwise =    → ["OK"; "OK"; "OK"]
n_NG := sum(check == "NG") =                        → 0
```

`check` has one verdict per pipe size, and `n_NG` counts the failures. Change the 3/4" `t_nom` to
`0.020` and the second verdict becomes "NG" and `n_NG` becomes 1. A sentence can quote `check`
("the checks are OK, NG, OK").

### Reserved words

`if`, `otherwise`, `and`, `or` and `not` can't be names, like the units and functions. A document
that already defined one of them as a variable shows `or is a reserved word — choose another name`
on that definition, and the same message on each calculation that uses the word as an operator,
rather than quietly changing what the document means. Rename the variable to fix it. A variable
named `text` is fine; only a heading ending in `(text)` is special.

### Conditions, text and sums in Word

Text, comparisons, cases (as a left brace, one line per case) and sums are saved as ordinary Word
math that reads correctly in Word, and reopen live in WordCraft. A text column's cells are an
ordinary Word table; if one is edited in Word, the new values are used and the status bar says so,
as for other calculation data.

## Function plots

Choose **Insert › Symbols › Function Plot** and enter the independent variable,
an expression such as `gain*x^2`, and a range such as `0, 0.1..10` or the name
of a numeric vector. The expression can use other live calculations and
functions in the document. Select the plot and choose **Chart Design › Edit
Function Plot** to change its sources. WordCraft saves the sources in DOCX.

## Formatting calculation results

In **References › Calculation › Options**, turn on **Customize the default
result format** to choose General, Decimal, Scientific, Engineering, or
Fraction for calculations in the document. To override that setting, right-click
a calculation and choose **Result Format**. Choose **Use document defaults**
to remove the override. WordCraft applies the number format after converting
to the selected display unit.

The worksheet importer preserves explicit number formats as overrides and
reads a worksheet default when the file supplies one. Both are kept when
saved as an editable DOCX.

## Importing a Mathcad worksheet

Open a Mathcad 14 or 15 `.xmcd` file or a Mathcad Prime `.mcdx` file with
**File › Open**. WordCraft creates an unsaved document. Save it as `.docx` to
keep the editable calculations. You can also run
`wordcraft-cli convert worksheet.mcdx worksheet.docx`.

The importer groups horizontally aligned regions into borderless tables with
as many columns as the row needs. Cells can span grid columns, and text keeps
its paragraph alignment and indents. Standalone regions remain paragraphs.
WordCraft recalculates the imported equations. It reads definitions, evaluations, functions, units, matrices,
ranges, and supported `Given`/`Find` blocks. Legacy global definitions (`~`)
become ordinary live definitions. Repeated definitions get distinct
names, such as `T_4b`, and later references use the new name. The original worksheet
is left alone.

The **Worksheet Appendix** gathers content from collapsed areas and from beyond
the right margin of worksheets set to print one page wide. It remains visible
and editable, and its formulas still evaluate in their original worksheet
order. The appendix starts on a new page. In **File › Print**, use a custom
page range ending before the appendix to print just the report.

The **Worksheet Import Review** opens after import. Select an entry to jump to
the document location for an unsupported region, calculation error, or saved
snapshot. While the import is open, use **References › Calculation › Worksheet
Import Review** to reopen it. An error with a single identified upstream cause can be expanded
under that cause. When the worksheet supplies a comparable unitless number,
the review shows the saved value beside the recalculated value at its display
precision.
The review does not validate units, arrays, or results absent from the source
file. Automation callers receive the entries as `importReview` in the
`file.open` result, with a document position for each entry.

When a legacy symbolic derivation or output-only component has a saved
picture, WordCraft can show that picture and identify it in Import Review.
Embedded metafile diagrams remain vector pictures. A saved picture keeps the
source worksheet's appearance, and its contents stay fixed when calculations
change.

A result can use a worksheet-defined scale as its display unit. For example, a
positive scalar definition of `inWG` with pressure units lets you write
`pressure = ? inWG`. WordCraft uses the value defined in the worksheet.

Periods in imported names become underscores, so `P_100.gage` becomes
`P_100_gage`. Adjacent and trailing separators are normalized, and a suffix keeps names
distinct if they would otherwise become identical.

Legacy `lb` is imported as pound-mass (`lbm`), and the display unit `g` maps
to standard gravity (`g_0`). Offset temperature literals can be imported as
interpolation vectors, and subtracting two absolute literals produces a
temperature difference. In imported `min` and `max` calls with a literal zero,
zero can take the dimension of the other values. Incompatible nonzero values
still produce a unit error.

Legacy `/FIF(length)` imports as `feetInches(length)`. It produces text such as
`6' 4"`, rounded to the nearest sixteenth of an inch. Use the original length
in formulas that need to keep calculating with units.

Mathcad's vectorize marker is removed for supported element-wise arithmetic.
Vectorized expressions that call functions such as `mean` are marked unsupported.

Review the imported results against Mathcad before using them. WordCraft uses
its own calculation and array rules, including element-wise multiplication
for arrays. The legacy importer handles function programs with local assignments,
indexed writes, conditional branches, bounded `for` loops, `break`, and `return`,
including nested coefficient vectors. Fixed matrix-cell assignments are kept in
worksheet order. Mixed-number dimensions and percentages are preserved, and
supported definitions embedded in text remain live. Unsupported programs, plots,
and Excel components are marked in the document. The import reflows the layout
and uses WordCraft's own results.
Prime imports include worksheet text with basic font styling, explicit page
breaks, and packaged PNG pictures. Prime arrays, solve blocks, and simple
conditional expressions use WordCraft's calculation engine. Prime headers and
footers are omitted, and complex programs and custom unit systems may require
manual conversion.

Legacy spreadsheet components can supply an import snapshot when the
worksheet contains an unambiguous saved matrix evaluation of the component's
output. The importer labels the recovered values. Mixed text-and-number
component matrices become document tables with a calculation binding when
their data fits the calculation limits. Their original cell types are
preserved, including text that resembles a number. Imported matrix column
selection and vertical lookup can use those bindings. The binding is a snapshot
stored with the calculation, so it survives saving as DOCX. Editing the displayed
table does not update the snapshot. When opening a
legacy worksheet from disk, WordCraft can resolve supported Excel `.xls` input
links from the worksheet's folder. If a coefficient sheet repeats its titled
block in a transposed copy, WordCraft can use the complete numeric copy when
the first block has no readable values. Relative links keep their directory structure.
An old absolute link such as `X:\WRC 537 Tables\C3.xls` is looked up as
`WRC 537 Tables/C3.xls` beside the worksheet. The importer reads a workbook with
one populated sheet and no explicit range selector in the link. Reopen the
original worksheet to read updated workbook values.

## Solving

A calculation can solve equations, as Mathcad's solve blocks do: you write the equations, give a
starting value for each unknown, and `Find` returns the values that make the equations hold. A
block is written as it is in Mathcad, so a report shows the governing equations and a Mathcad
solve block maps over line for line.

### Typing a block

A block is three kinds of calculation, one under the other:

1. `Given`, alone in its calculation.
2. Optional starting values for the unknowns, such as `f := 0.02`. Starting values can also go above `Given`.
3. The equations, each written `left == right` (`==` is shown as the bold `=`).
4. A calculation that ends the block with `Find(…)`.

The Colebrook friction factor in a pipe, solved for `f`:

```
Re := 200000
ε_r := 0.0002
f := 0.02
Given
1/√f == −2·log(ε_r/3.7 + 2.51/(Re·√f))
f_D := Find(f) =                            → 0.017
```

The results here are shown at the default three decimal places (see
[How results are shown](#how-results-are-shown)); `f_D` is 0.017098 to six.

- The calculation that ends a block is the first one whose whole right side is `Find(…)` or
  `Minerr(…)`: `name := Find(…)`, `name(params) := Find(…)`, `[a; b] := Find(…)` or `Find(…) =`.
  `Find`'s arguments are the unknowns, written as bare names: `Find(f)`, `Find(x, y)`.
- `Given` and the equations show no result. An error shows on them only when the block is wrong.
- A block is the one place in a document where order matters. Everywhere else a calculation can
  still be above or below what it uses. The equations can use any name defined anywhere, and the
  calculation that ends the block updates when any of them changes.
- Inside the equations an unknown's name means the unknown being solved for, not its starting
  value.
- Only equations go between `Given` and `Find`. Put ordinary definitions and results above
  `Given` or below the block.
- A calculation inside a tracked deletion is ignored: it doesn't open, close or join a block.
- The sides of an equation must be single numbers of the same dimension (`p == z` with psi on one
  side and ft on the other is the usual unit error, on the calculation that ends the block).

### Starting values

Each unknown needs an ordinary definition somewhere in the document: one number, with or without a
unit. It is the value the solve starts from, and it sets the unknown's dimension, so a guess in
`ft` gives an answer in `ft`.

- Guesses are shared across the document. Two blocks that solve for `x` start from the same `x`.
  To start a second block from another value, give its unknown another name.
- The guess keeps its own name and value everywhere else. Showing `f =` after the block above
  shows 0.02, not the answer. So the answer needs a new name: `x := Find(x)` is a duplicate
  definition, as any name defined twice is. Mathcad worksheets often do this, so rename the result
  when you bring one over.
- A guess of zero is fine (`V_1 := 0 ft/sec`).
- Inside a function, an unknown that is also one of the function's parameters starts from the
  argument it was called with, as in Mathcad: `q(x) := root(x^2 − 2, x)` starts from `x`.
- A function called from the equations sees the guess, not the unknown: with
  `shift(a) := a + x`, the equation `shift(1) == 5` reads the document's `x`, whatever value the
  solve is trying. Write the unknown into the equation itself, or pass it in (`shift(1, x)` with
  `shift(a, x) := a + x`).

### Several unknowns

`Find(x, y)` gives a column vector, in the order listed. Its elements can have different units.

```
x := 1
y := 1
Given
x + y == 5
x − y == 1
[x_s; y_s] := Find(x, y) =                  → [3; 2]
```

`[x_s; y_s] := …` defines each name from the matching element. It isn't limited to `Find`: the
right side can be any expression that gives a column vector of that length. The names follow the
usual rules, and one that is already defined is the usual duplicate error. A solve can have up to
50 unknowns.

There can be more equations than unknowns, as long as they agree. With too few equations to fix
the answer, `Find` returns the answer nearest the starting values, as Mathcad does.

### `Minerr`

`Minerr(x, y)` is written like `Find`, but it returns the closest fit instead of requiring a
solution. It minimizes the sum of the squared differences between the two sides of each equation,
taken in SI base units (which is how Mathcad holds values), so mixed units don't favor one
equation.

```
x := 1
y := 1
Given
x + y == 3
x − y == 1
x == 2.2
[x_m; y_m] := Minerr(x, y) =                → [2.067; 1]
```

`Find` returns only when every equation holds. `Minerr` is an error only when the solve can't
settle on an answer or the answer isn't a finite number.

### `root`

`root(expression, x)` solves `expression = 0` for `x`, starting from `x`'s guess, and works
anywhere an expression can, including in a function body:

```
x := 1
x_r := root(x^2 − 2, x) =                   → 1.414
q(a) := root(x^2 − a, x)
q(9) =                                      → 3
```

Inside the expression, `x` is the unknown. `root` has the same rules for guesses, dimensions and
an answer that isn't a root as `Find`. It has no search range (`root(f, x, a, b)`) yet.

### Blocks as functions

A block can be a function of the values it needs, which is how the Colebrook factor is written in a
pump worksheet:

```
f := 0.02
Given
1/√f == −2·log(ε_r/3.7 + 2.51/(R·√f))
f_turb(R, ε_r) := Find(f)
f_turb(200000, 0.0002) =                    → 0.017
```

Each call solves again with the arguments it is given. The starting value is still the document's
`f`. `R` and `ε_r` are parameters here, so they need no definitions of their own.

A call with a vector solves the whole vector at once when the equations allow it. When they need
single numbers, as these do, the function solves once for each element:

```
f_turb([1e5; 2e5; 1e6], 0.0002) =          → [0.019; 0.017; 0.015]
```

- An element with no solution is an error that names it ("element 1: …").
- In a case branch worked out per element (see [Cases](#cases)), only the failing element fails.
  The other elements keep their results.
- A function with several unknowns takes single values; it can't be called with a vector.
- Each element's solve counts against the document's work limit.

### Accuracy

WordCraft solves to nearly full double precision; Mathcad stops at its default tolerance of
0.001. A WordCraft answer can therefore differ from a Mathcad saved result in the fourth or fifth
significant figure by being more exact.

| Setting | Value |
| --- | --- |
| An equation holds when its two sides differ by no more than | 1e-10 of the equation's scale |
| The equation's scale, at the answer | the largest of: each side's size; how much the difference of the sides changes when each unknown changes by its own size |
| An unknown's own size | its size at the answer, but at least 1e-10 of its starting value (1e-10 when that is 0) |
| The most steps one solve takes (`MAX_SOLVE_STEPS`) | 200 |
| The most unknowns in one solve | 50 |

The scale is measured at the answer only, never at the starting values, so a difference that
looks small next to huge sides at the guess (or next to a huge guess) doesn't pass. The scale
makes the test work the same in psi or in Pa, and keeps `… == 0` and `root` meaningful, where
one side is exactly 0: there the sides are judged against how much the unknowns move them. An
unknown that has gone to 0 is measured against its starting value, so a root at 0 still holds.
Where the solve stopped because no change of the unknowns makes the sides any closer (equations
that contradict each other), only the sides' own sizes count, so a near miss there doesn't pass.
Sides that are exactly equal always hold.

Repeated roots, such as `(x - 2)^2 == 0`, can be rejected if the solver stops
before it reaches the root. A very large difference in scale between the starting
guess and the expected answer can also make the result unreliable. Check the
equations at the returned value in those cases.

If `Find` runs out of steps (200) at a point where every equation already holds, it returns that
point; otherwise it is the "didn't converge" error.

`Find` checks every equation against the tolerance before it returns. Every try of the
equations also counts against the limit on a
document's work, so a solve can't freeze the document: a solve that reaches that limit shows the
usual "too complex to calculate" error (or "too complex", for one nested too deeply), not "didn't
converge".

To check a result by hand, put the answer back into an equation: `lhs − rhs =` after the block,
using the name you gave the answer. WordCraft's own tests check Mathcad's saved results to three
significant figures: a result agrees when it is within half a unit of the third significant
figure of Mathcad's, so 207.6 ft passes against 207.61 ft and 208.2 ft fails.

### Errors

| Message | What it means |
| --- | --- |
| a Given block accepts only starting values for its unknowns followed by equations | A calculation inside the block is neither an equation nor a starting value for a listed unknown before the equations. It shows on that calculation once the block closes. |
| an equation needs a Given above it | An `==` calculation with no open block above it. |
| this Given has no Find or Minerr below it to end its block | A `Given` that nothing ends, before the end of the document or the next `Given`. It shows on the `Given` alone: other calculations below it work as usual, and its equations show nothing. A second `Given` starts a block of its own. |
| a Given block needs at least one equation | `Given` followed straight by `Find`. |
| Find must end a Given block | A `Find` with no `Given` above it. |
| Minerr must end a Given block | A `Minerr` with no `Given` above it. |
| Find must be the whole right side of the line that ends a Given block | `Find` (or `Minerr`) inside an expression, like `x_s := 2·Find(x)`. Solve first, then use the answer: `x_s := Find(x)`, then `2·x_s`. |
| Find's arguments are the unknowns, like Find(x, y) | `Find` was given something other than names. |
| x is listed twice | The same unknown appears twice in `Find(…)` or `Minerr(…)`. |
| give f a starting value | The unknown `f` has no definition to start from. |
| f's starting value must be one number — define it like f := 1 | The definition is a vector, not one number. |
| f is a function — call it like f(x) | The unknown's name is a function, not a number to start from. |
| a solve works on single numbers — a vector can't be an unknown or a side of an equation | An unknown or a side of an equation is a vector. |
| a Find with several unknowns takes single values — it can't be called with a vector | A block with several unknowns, used as a function, was called with a vector (see [Blocks as functions](#blocks-as-functions)). |
| equation 2: … | The second equation of the block (counted from 1, in document order) has a unit error, like psi on one side and ft on the other. It shows on the calculation that ends the block. |
| [a; b] takes a column of 2 values, but the right side has 3 | The names on the left of `[a; b] := …` don't match the length of the column on the right. |
| no solution found near the guesses — the equations are still off by …; try other guesses | The solve stopped where an equation still doesn't hold. The message says by how much, against the equation's scale (see [Accuracy](#accuracy)). Try other starting values. `root` shows the same message. |
| the solve didn't converge in N steps — try other guesses | The solve ran out of steps. |
| root expects 2 argument(s) | `root` takes the expression and the unknown, `root(x^2 − 2, x)`. |
| root's second argument is the unknown, like root(x^2 − 2, x) | `root`'s second argument isn't a name. |
| a solve can have at most 50 unknowns | Too many names in `Find`. |
| too complex to calculate — the document's calculations need more work than one recalculation allows | The solve, with the rest of the document, needs more work than one recalculation may do. |
| too complex | The solve is nested too deeply (a block used as a function inside a block, many levels down). |
| depends on a calculation edited in Word — resolve it | An equation in the block was edited in Word (see [Solving in Word](#solving-in-word)). |

An error on any equation or on the guess makes the calculation that ends the block fail, and every
calculation that uses the answer shows "depends on a calculation with an error".

### Solving in Word

A block saves and reopens like any calculation: `Given`, the equations and `Find` come back as
they were, with the same results. If an equation or `Given` is edited in Word, it is flagged with
the amber ⚠ as any edited calculation is, and the calculation that ends its block fails with
"depends on a calculation edited in Word — resolve it" until you restore it or keep Word's
version.

### Not supported yet

- Inequalities in a block (`≤`, `≥`, `<`, `>`).
- `Maximize` and `Minimize`, `Odesolve`, symbolic solving and `polyroots`.
- `root` with a search range, `root(f, x, a, b)`.
- Vectors or matrices as unknowns. Each unknown is one value; several give a vector.
- `TOL` and `CTOL`. In WordCraft these are ordinary names.

### Reserved names

`Given`, `Find`, `Minerr` and `root` can't be variable or function names. A document that already
defined one shows the reserved-name error on that definition, and every calculation that uses the
name shows "depends on a calculation with an error" until it is renamed: the name never quietly
changes what it means.

## Example

A bolt-stress check, as exercised by the engine's `flange_bolt_sample` test:

```
n_b := 8
d_b := 0.75 in
F_i := 20 kip
σ_allow := 45 ksi
A_t(d) := π/4·d^2
σ_b := F_i/A_t(d_b) = ? ksi        → 45.271 ksi
F_total := n_b·F_i = ? kip         → 160 kip
Each bolt carries [σ_b in ksi] of stress.
```

Changing `F_i` to `18 kip` updates both results and the sentence (40.744 ksi, 144 kip).

For worked examples with a data table (an ASME B31.3 wall thickness check and a flange torque
table), see [Data tables](#data-tables).

## Building

The calculation engine uses the `craft-units` crate from craft-libs. It is pinned in the workspace
`Cargo.toml` to a commit on `alanrathbun/craft-libs` (branch `feature/craft-units`), so a plain
`cargo build` fetches it; no separate checkout is needed. Move the pin to `storytold/craft-libs`
if the crate is accepted there.
