//! The Equation tab's galleries: structure templates (fractions, scripts, radicals, integrals,
//! large operators, brackets, functions, accents, limits and logs, operators, matrices), symbol
//! sets and ready-made equations. Every template is linear-format text (`⬚` = empty slot) that
//! [`wordcraft_doc::math::parse_linear`] builds into structures.

use wordcraft_doc::math::{Arg, LimLoc, parse_linear};

/// One template.
#[derive(Clone, Copy, Debug)]
pub struct Template {
    pub id: &'static str,
    pub label: &'static str,
    /// Linear format; `⬚` marks an empty argument.
    pub linear: &'static str,
    /// Limit placement for n-ary operators (`None`: the operator's default).
    pub limits: Option<LimLoc>,
}

const fn t(id: &'static str, label: &'static str, linear: &'static str) -> Template {
    Template { id, label, linear, limits: None }
}
const fn stacked(id: &'static str, label: &'static str, linear: &'static str) -> Template {
    Template { id, label, linear, limits: Some(LimLoc::UndOvr) }
}
const fn beside(id: &'static str, label: &'static str, linear: &'static str) -> Template {
    Template { id, label, linear, limits: Some(LimLoc::SubSup) }
}

impl Template {
    /// The template as nodes.
    pub fn nodes(&self) -> Arg {
        let mut n = parse_linear(self.linear);
        if self.limits.is_some() {
            wordcraft_doc::math_edit::set_limits(&mut n, self.limits);
        }
        n
    }
}

/// A structure gallery (one Equation tab button) with its sections.
pub struct Gallery {
    pub id: &'static str,
    pub label: &'static str,
    pub sections: &'static [(&'static str, &'static [Template])],
}

pub const STRUCTURES: &[Gallery] = &[
    Gallery {
        id: "fraction",
        label: "Fraction",
        sections: &[
            (
                "Fraction",
                &[
                    t("frac.stacked", "Stacked Fraction", "⬚/⬚"),
                    t("frac.skewed", "Skewed Fraction", "⬚⁄⬚"),
                    t("frac.linear", "Linear Fraction", "⬚∕⬚"),
                    t("frac.nobar", "Stacked, no bar", "⬚¦⬚"),
                ],
            ),
            (
                "Common Fractions",
                &[
                    t("frac.dydx", "Differential", "dy/dx"),
                    t("frac.DyDx", "Change in y over change in x", "Δy/Δx"),
                    t("frac.partial", "Partial Differential", "∂y/∂x"),
                    t("frac.delta", "Small change", "δy/δx"),
                    t("frac.pi2", "Pi Over 2", "π/2"),
                    t("frac.half", "One half", "1/2"),
                ],
            ),
        ],
    },
    Gallery {
        id: "script",
        label: "Script",
        sections: &[
            (
                "Subscripts and Superscripts",
                &[
                    t("script.sup", "Superscript", "⬚^⬚"),
                    t("script.sub", "Subscript", "⬚_⬚"),
                    t("script.subsup", "Subscript-Superscript", "⬚_⬚^⬚"),
                    t("script.pre", "Left Subscript-Superscript", "_⬚^⬚▒⬚"),
                ],
            ),
            (
                "Common Subscripts and Superscripts",
                &[
                    t("script.xy2", "Subscript-Superscript", "x_y^2"),
                    t("script.eiwt", "Exponential", "e^(−iωt)"),
                    t("script.x2", "Square", "x^2"),
                    t("script.pre1n", "Left Subscript-Superscript", "_1^n▒Y"),
                ],
            ),
        ],
    },
    Gallery {
        id: "radical",
        label: "Radical",
        sections: &[
            (
                "Radicals",
                &[
                    t("rad.sqrt", "Square Root", "√⬚"),
                    t("rad.nth", "Radical with Degree", "√(⬚&⬚)"),
                    t("rad.sqrt2", "Square Root with Degree", "√(2&⬚)"),
                    t("rad.cbrt", "Cube Root", "√(3&⬚)"),
                ],
            ),
            ("Common Radicals", &[t("rad.quadratic", "Quadratic Root", "(−b±√(b^2−4ac))/2a"), t("rad.hyp", "Hypotenuse", "√(a^2+b^2)")]),
        ],
    },
    Gallery {
        id: "integral",
        label: "Integral",
        sections: &[
            (
                "Integrals",
                &[
                    t("int.plain", "Integral", "∫▒⬚"),
                    beside("int.limits", "Integral with Limits", "∫_⬚^⬚▒⬚"),
                    stacked("int.stacked", "Integral with Stacked Limits", "∫_⬚^⬚▒⬚"),
                    t("int.double", "Double Integral", "∬▒⬚"),
                    beside("int.doubleLimits", "Double Integral with Limits", "∬_⬚^⬚▒⬚"),
                    stacked("int.doubleStacked", "Double Integral with Stacked Limits", "∬_⬚^⬚▒⬚"),
                    t("int.triple", "Triple Integral", "∭▒⬚"),
                    beside("int.tripleLimits", "Triple Integral with Limits", "∭_⬚^⬚▒⬚"),
                    stacked("int.tripleStacked", "Triple Integral with Stacked Limits", "∭_⬚^⬚▒⬚"),
                ],
            ),
            (
                "Contour Integrals",
                &[
                    t("int.contour", "Contour Integral", "∮▒⬚"),
                    beside("int.contourLimits", "Contour Integral with Limits", "∮_⬚^⬚▒⬚"),
                    stacked("int.contourStacked", "Contour Integral with Stacked Limits", "∮_⬚^⬚▒⬚"),
                    t("int.surface", "Surface Integral", "∯▒⬚"),
                    beside("int.surfaceLimits", "Surface Integral with Limits", "∯_⬚^⬚▒⬚"),
                    stacked("int.surfaceStacked", "Surface Integral with Stacked Limits", "∯_⬚^⬚▒⬚"),
                    t("int.volume", "Volume Integral", "∰▒⬚"),
                    beside("int.volumeLimits", "Volume Integral with Limits", "∰_⬚^⬚▒⬚"),
                    stacked("int.volumeStacked", "Volume Integral with Stacked Limits", "∰_⬚^⬚▒⬚"),
                ],
            ),
            (
                "Differentials",
                &[t("int.dx", "Differential x", "dx"), t("int.dy", "Differential y", "dy"), t("int.dtheta", "Differential theta", "dθ")],
            ),
        ],
    },
    Gallery {
        id: "largeOperator",
        label: "Large\nOperator",
        sections: &[
            (
                "Summations",
                &[
                    t("nary.sum", "Summation", "∑▒⬚"),
                    stacked("nary.sumLimits", "Summation with Limits", "∑_⬚^⬚▒⬚"),
                    beside("nary.sumScripts", "Summation with Subscript-Superscript Limits", "∑_⬚^⬚▒⬚"),
                    stacked("nary.sumLower", "Summation with Lower Limit", "∑_⬚▒⬚"),
                    beside("nary.sumSub", "Summation with Subscript Limit", "∑_⬚▒⬚"),
                ],
            ),
            (
                "Products and Coproducts",
                &[
                    t("nary.prod", "Product", "∏▒⬚"),
                    stacked("nary.prodLimits", "Product with Limits", "∏_⬚^⬚▒⬚"),
                    beside("nary.prodScripts", "Product with Subscript-Superscript Limits", "∏_⬚^⬚▒⬚"),
                    stacked("nary.prodLower", "Product with Lower Limit", "∏_⬚▒⬚"),
                    t("nary.coprod", "Coproduct", "∐▒⬚"),
                    stacked("nary.coprodLimits", "Coproduct with Limits", "∐_⬚^⬚▒⬚"),
                    beside("nary.coprodScripts", "Coproduct with Subscript-Superscript Limits", "∐_⬚^⬚▒⬚"),
                    stacked("nary.coprodLower", "Coproduct with Lower Limit", "∐_⬚▒⬚"),
                ],
            ),
            (
                "Unions and Intersections",
                &[
                    t("nary.union", "Union", "⋃▒⬚"),
                    stacked("nary.unionLimits", "Union with Limits", "⋃_⬚^⬚▒⬚"),
                    beside("nary.unionScripts", "Union with Subscript-Superscript Limits", "⋃_⬚^⬚▒⬚"),
                    stacked("nary.unionLower", "Union with Lower Limit", "⋃_⬚▒⬚"),
                    t("nary.intersection", "Intersection", "⋂▒⬚"),
                    stacked("nary.intersectionLimits", "Intersection with Limits", "⋂_⬚^⬚▒⬚"),
                    beside("nary.intersectionScripts", "Intersection with Subscript-Superscript Limits", "⋂_⬚^⬚▒⬚"),
                    stacked("nary.intersectionLower", "Intersection with Lower Limit", "⋂_⬚▒⬚"),
                ],
            ),
            (
                "Other Large Operators",
                &[
                    t("nary.or", "Logical OR", "⋁▒⬚"),
                    stacked("nary.orLimits", "Logical OR with Limits", "⋁_⬚^⬚▒⬚"),
                    t("nary.and", "Logical AND", "⋀▒⬚"),
                    stacked("nary.andLimits", "Logical AND with Limits", "⋀_⬚^⬚▒⬚"),
                    t("nary.oplus", "Circled Plus", "⨁▒⬚"),
                    t("nary.otimes", "Circled Times", "⨂▒⬚"),
                    t("nary.odot", "Circled Dot", "⨀▒⬚"),
                    t("nary.uplus", "Union with Plus", "⨄▒⬚"),
                    t("nary.sqcup", "Square Union", "⨆▒⬚"),
                ],
            ),
            (
                "Common Large Operators",
                &[
                    stacked("nary.binomial", "Summation over binomials", "∑_k▒〖(n¦k)〗"),
                    stacked("nary.sum0n", "Summation from 0 to n", "∑_(i=0)^n▒⬚"),
                    stacked("nary.double", "Double summation", "∑_(0≤i≤m,0<j<n)▒P(i,j)"),
                    stacked("nary.prodA", "Product of A", "∏_(k=1)^n▒A_k"),
                    stacked("nary.unionXY", "Union of intersections", "⋃_(n=1)^m▒〖(X_n∩Y_n)〗"),
                ],
            ),
        ],
    },
    Gallery {
        id: "bracket",
        label: "Bracket",
        sections: &[
            (
                "Brackets",
                &[
                    t("brk.paren", "Parentheses", "(⬚)"),
                    t("brk.square", "Brackets", "[⬚]"),
                    t("brk.brace", "Braces", "{⬚}"),
                    t("brk.angle", "Angle Brackets", "⟨⬚⟩"),
                    t("brk.floor", "Floor", "⌊⬚⌋"),
                    t("brk.ceil", "Ceiling", "⌈⬚⌉"),
                    t("brk.abs", "Vertical Bars", "|⬚|"),
                    t("brk.norm", "Double Vertical Bars", "‖⬚‖"),
                    t("brk.dbl", "Double Square Brackets", "⟦⬚⟧"),
                    t("brk.leftSquare", "Left Bracket Twice", "├[⬚[┤"),
                    t("brk.rightSquare", "Right Bracket Twice", "├]⬚]┤"),
                    t("brk.outward", "Outward Brackets", "├]⬚[┤"),
                ],
            ),
            (
                "Brackets with Separators",
                &[
                    t("brk.parenSep", "Parentheses with Separator", "(⬚│⬚)"),
                    t("brk.braceSep", "Braces with Separator", "{⬚│⬚}"),
                    t("brk.angleSep", "Angle Brackets with Separator", "⟨⬚│⬚⟩"),
                    t("brk.angleSep2", "Angle Brackets with Two Separators", "⟨⬚│⬚│⬚⟩"),
                ],
            ),
            (
                "Single Brackets",
                &[
                    t("brk.leftParen", "Single Left Parenthesis", "(⬚┤"),
                    t("brk.rightParen", "Single Right Parenthesis", "├⬚)"),
                    t("brk.leftSquareOnly", "Single Left Bracket", "[⬚┤"),
                    t("brk.rightSquareOnly", "Single Right Bracket", "├⬚]"),
                    t("brk.leftBrace", "Single Left Brace", "{⬚┤"),
                    t("brk.rightBrace", "Single Right Brace", "├⬚}"),
                    t("brk.leftBar", "Single Left Bar", "├|⬚┤"),
                    t("brk.leftAngle", "Single Left Angle", "⟨⬚┤"),
                ],
            ),
            (
                "Cases and Stacks",
                &[
                    t("brk.cases2", "Cases (Two Conditions)", "{█(⬚@⬚)┤"),
                    t("brk.cases3", "Cases (Three Conditions)", "{█(⬚@⬚@⬚)┤"),
                    t("brk.stack2", "Stack Object", "█(⬚@⬚)"),
                    t("brk.stack3", "Stack Object (Three)", "█(⬚@⬚@⬚)"),
                    t("brk.binom", "Binomial Coefficient", "(⬚¦⬚)"),
                    t("brk.binomAngle", "Binomial Coefficient with Angle Brackets", "⟨⬚¦⬚⟩"),
                ],
            ),
            (
                "Common Brackets",
                &[
                    t("brk.casesAbs", "Absolute value cases", "f(x)={█(−x,&x<0@x,&x≥0)┤"),
                    t("brk.nk", "Binomial Coefficient", "(n¦k)"),
                    t("brk.nk2", "Binomial Coefficient (angle)", "⟨n¦k⟩"),
                ],
            ),
        ],
    },
    Gallery {
        id: "function",
        label: "Function",
        sections: &[
            (
                "Trigonometric Functions",
                &[
                    t("fn.sin", "Sine Function", "sin⁡⬚"),
                    t("fn.cos", "Cosine Function", "cos⁡⬚"),
                    t("fn.tan", "Tangent Function", "tan⁡⬚"),
                    t("fn.csc", "Cosecant Function", "csc⁡⬚"),
                    t("fn.sec", "Secant Function", "sec⁡⬚"),
                    t("fn.cot", "Cotangent Function", "cot⁡⬚"),
                ],
            ),
            (
                "Inverse Functions",
                &[
                    t("fn.asin", "Inverse Sine Function", "sin^(−1)⁡⬚"),
                    t("fn.acos", "Inverse Cosine Function", "cos^(−1)⁡⬚"),
                    t("fn.atan", "Inverse Tangent Function", "tan^(−1)⁡⬚"),
                    t("fn.acsc", "Inverse Cosecant Function", "csc^(−1)⁡⬚"),
                    t("fn.asec", "Inverse Secant Function", "sec^(−1)⁡⬚"),
                    t("fn.acot", "Inverse Cotangent Function", "cot^(−1)⁡⬚"),
                ],
            ),
            (
                "Hyperbolic Functions",
                &[
                    t("fn.sinh", "Hyperbolic Sine Function", "sinh⁡⬚"),
                    t("fn.cosh", "Hyperbolic Cosine Function", "cosh⁡⬚"),
                    t("fn.tanh", "Hyperbolic Tangent Function", "tanh⁡⬚"),
                    t("fn.csch", "Hyperbolic Cosecant Function", "csch⁡⬚"),
                    t("fn.sech", "Hyperbolic Secant Function", "sech⁡⬚"),
                    t("fn.coth", "Hyperbolic Cotangent Function", "coth⁡⬚"),
                ],
            ),
            (
                "Inverse Hyperbolic Functions",
                &[
                    t("fn.asinh", "Inverse Hyperbolic Sine Function", "sinh^(−1)⁡⬚"),
                    t("fn.acosh", "Inverse Hyperbolic Cosine Function", "cosh^(−1)⁡⬚"),
                    t("fn.atanh", "Inverse Hyperbolic Tangent Function", "tanh^(−1)⁡⬚"),
                    t("fn.acsch", "Inverse Hyperbolic Cosecant Function", "csch^(−1)⁡⬚"),
                    t("fn.asech", "Inverse Hyperbolic Secant Function", "sech^(−1)⁡⬚"),
                    t("fn.acoth", "Inverse Hyperbolic Cotangent Function", "coth^(−1)⁡⬚"),
                ],
            ),
            (
                "Common Functions",
                &[
                    t("fn.sintheta", "Sine theta", "sin⁡θ"),
                    t("fn.cos2x", "Cosine 2x", "cos⁡2x"),
                    t("fn.tanIdentity", "Tangent formula", "tan⁡θ=sin⁡θ/cos⁡θ"),
                ],
            ),
        ],
    },
    Gallery {
        id: "accent",
        label: "Accent",
        sections: &[
            (
                "Accents",
                &[
                    t("acc.dot", "Dot", "⬚\u{307}"),
                    t("acc.ddot", "Double Dot", "⬚\u{308}"),
                    t("acc.dddot", "Triple Dot", "⬚\u{20DB}"),
                    t("acc.hat", "Hat", "⬚\u{302}"),
                    t("acc.check", "Check", "⬚\u{30C}"),
                    t("acc.acute", "Acute", "⬚\u{301}"),
                    t("acc.grave", "Grave", "⬚\u{300}"),
                    t("acc.breve", "Breve", "⬚\u{306}"),
                    t("acc.tilde", "Tilde", "⬚\u{303}"),
                    t("acc.bar", "Bar", "⬚\u{305}"),
                    t("acc.dbar", "Double Bar", "⬚\u{33F}"),
                    t("acc.overbrace", "Overbrace", "⏞⬚"),
                    t("acc.underbrace", "Underbrace", "⏟⬚"),
                    t("acc.braceAbove", "Grouping Character Above", "⏞(⬚)┴⬚"),
                    t("acc.braceBelow", "Grouping Character Below", "⏟(⬚)┬⬚"),
                    t("acc.leftArrow", "Leftwards Arrow Above", "⬚\u{20D6}"),
                    t("acc.rightArrow", "Rightwards Arrow Above", "⬚\u{20D7}"),
                    t("acc.bothArrow", "Left Right Arrow Above", "⬚\u{20E1}"),
                    t("acc.leftHarpoon", "Leftwards Harpoon Above", "⬚\u{20D0}"),
                    t("acc.rightHarpoon", "Rightwards Harpoon Above", "⬚\u{20D1}"),
                ],
            ),
            ("Boxed Formulas", &[t("acc.boxed", "Boxed Formula", "▭(⬚)"), t("acc.boxedPythagoras", "Boxed Formula (example)", "▭(a^2=b^2+c^2)")]),
            ("Overbars and Underbars", &[t("acc.overbar", "Overbar", "¯(⬚)"), t("acc.underbar", "Underbar", "▁(⬚)")]),
            (
                "Common Accent Objects",
                &[
                    t("acc.vecA", "Vector A", "A\u{20D7}"),
                    t("acc.vecAB", "Vector AB", "(AB)\u{20E1}"),
                    t("acc.xbar", "x bar", "x\u{305}"),
                    t("acc.notAxorB", "A xor B", "¯(A⊕B)"),
                ],
            ),
        ],
    },
    Gallery {
        id: "limitLog",
        label: "Limit and\nLog",
        sections: &[
            (
                "Functions",
                &[
                    t("lim.logBase", "Logarithm with Base", "log_⬚⁡⬚"),
                    t("lim.log", "Logarithm", "log⁡⬚"),
                    t("lim.lim", "Limit", "lim┬⬚⁡⬚"),
                    t("lim.min", "Minimum", "min┬⬚⁡⬚"),
                    t("lim.max", "Maximum", "max┬⬚⁡⬚"),
                    t("lim.ln", "Natural Logarithm", "ln⁡⬚"),
                ],
            ),
            (
                "Common Functions",
                &[t("lim.e", "Limit Example", "lim┬(n→∞)⁡〖(1+1/n)^n〗"), t("lim.maxExample", "Maximum Example", "max┬(0≤x≤1)⁡〖xe^(−x^2)〗")],
            ),
        ],
    },
    Gallery {
        id: "operator",
        label: "Operator",
        sections: &[
            (
                "Common Operators",
                &[
                    t("op.colonEq", "Colon Equal", "≔"),
                    t("op.eqeq", "Equal Equal", "=="),
                    t("op.plusEq", "Plus Equal", "+="),
                    t("op.minusEq", "Minus Equal", "−="),
                    t("op.defEq", "Equal by Definition", "≝"),
                    t("op.measured", "Measured By", "≞"),
                    t("op.delta", "Delta Equal To", "≜"),
                ],
            ),
            (
                "Operator Structures",
                &[
                    t("op.leftArrowBelow", "Left Arrow with Text Below", "⟵┬⬚"),
                    t("op.rightArrowBelow", "Right Arrow with Text Below", "⟶┬⬚"),
                    t("op.leftArrowAbove", "Left Arrow with Text Above", "⟵┴⬚"),
                    t("op.rightArrowAbove", "Right Arrow with Text Above", "⟶┴⬚"),
                    t("op.LeftArrowBelow", "Double Left Arrow with Text Below", "⟸┬⬚"),
                    t("op.RightArrowBelow", "Double Right Arrow with Text Below", "⟹┬⬚"),
                    t("op.LeftArrowAbove", "Double Left Arrow with Text Above", "⟸┴⬚"),
                    t("op.RightArrowAbove", "Double Right Arrow with Text Above", "⟹┴⬚"),
                    t("op.bothArrowBelow", "Left Right Arrow with Text Below", "⟷┬⬚"),
                    t("op.bothArrowAbove", "Left Right Arrow with Text Above", "⟷┴⬚"),
                    t("op.BothArrowBelow", "Double Left Right Arrow with Text Below", "⟺┬⬚"),
                    t("op.BothArrowAbove", "Double Left Right Arrow with Text Above", "⟺┴⬚"),
                    t("op.colonEqAbove", "Text over Colon Equal", "≔┴⬚"),
                    t("op.eqAbove", "Text over Equal", "=┴⬚"),
                ],
            ),
            ("Common Operator Structures", &[t("op.yields", "Yields", "⟶┴(yields)"), t("op.deltaArrow", "Delta over arrow", "⟶┴∆")]),
        ],
    },
    Gallery {
        id: "matrix",
        label: "Matrix",
        sections: &[
            (
                "Empty Matrices",
                &[
                    t("mat.1x2", "1×2 Empty Matrix", "■(⬚&⬚)"),
                    t("mat.2x1", "2×1 Empty Matrix", "■(⬚@⬚)"),
                    t("mat.1x3", "1×3 Empty Matrix", "■(⬚&⬚&⬚)"),
                    t("mat.3x1", "3×1 Empty Matrix", "■(⬚@⬚@⬚)"),
                    t("mat.2x2", "2×2 Empty Matrix", "■(⬚&⬚@⬚&⬚)"),
                    t("mat.2x3", "2×3 Empty Matrix", "■(⬚&⬚&⬚@⬚&⬚&⬚)"),
                    t("mat.3x2", "3×2 Empty Matrix", "■(⬚&⬚@⬚&⬚@⬚&⬚)"),
                    t("mat.3x3", "3×3 Empty Matrix", "■(⬚&⬚&⬚@⬚&⬚&⬚@⬚&⬚&⬚)"),
                ],
            ),
            (
                "Dots",
                &[
                    t("mat.cdots", "Midline Dots", "⋯"),
                    t("mat.ldots", "Baseline Dots", "…"),
                    t("mat.vdots", "Vertical Dots", "⋮"),
                    t("mat.ddots", "Diagonal Dots", "⋱"),
                    t("mat.adots", "Up Diagonal Dots", "⋰"),
                ],
            ),
            (
                "Identity Matrices",
                &[
                    t("mat.id2", "2×2 Identity Matrix", "■(1&0@0&1)"),
                    t("mat.id2zeros", "2×2 Identity Matrix (sparse)", "■(1&⬚@⬚&1)"),
                    t("mat.id3", "3×3 Identity Matrix", "■(1&0&0@0&1&0@0&0&1)"),
                    t("mat.id3zeros", "3×3 Identity Matrix (sparse)", "■(1&⬚&⬚@⬚&1&⬚@⬚&⬚&1)"),
                ],
            ),
            (
                "Matrices with Brackets",
                &[
                    t("mat.paren", "2×2 Matrix in Parentheses", "(■(⬚&⬚@⬚&⬚))"),
                    t("mat.square", "2×2 Matrix in Brackets", "[■(⬚&⬚@⬚&⬚)]"),
                    t("mat.det", "2×2 Determinant", "|■(⬚&⬚@⬚&⬚)|"),
                    t("mat.norm", "2×2 Matrix in Double Bars", "‖■(⬚&⬚@⬚&⬚)‖"),
                ],
            ),
            (
                "Sparse Matrices",
                &[
                    t("mat.sparse", "n×n Matrix", "(■(a_11&⋯&a_(1n)@⋮&⋱&⋮@a_(m1)&⋯&a_(mn)))"),
                    t("mat.identityN", "n×n Identity Matrix", "[■(1&0&⋯&0@0&1&⋯&0@⋮&⋮&⋱&⋮@0&0&⋯&1)]"),
                ],
            ),
        ],
    },
];

/// Every structure template.
pub fn templates() -> impl Iterator<Item = &'static Template> {
    STRUCTURES.iter().flat_map(|g| g.sections.iter().flat_map(|(_, ts)| ts.iter()))
}

pub fn template(id: &str) -> Option<&'static Template> {
    templates().find(|t| t.id == id)
}

/// Symbol sets (the Symbols gallery's categories).
pub const SYMBOL_SETS: &[(&str, &str)] = &[
    ("Basic Math", "±∞=≠~×÷!∝<≪>≫≤≥∓≅≈≡∀∁∂√∛∜∪∩∅%°℉℃∆∇∃∄∈∋←↑→↓↔∴+−¬αβγδεϵθϑμπρστφωω∗∙⋮⋯⋰⋱ℵℶ∎"),
    ("Greek Letters", "αβγδεϵζηθϑικλμνξοπϖρϱσςτυφϕχψωΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ"),
    ("Letter-Like Symbols", "∀∁ℂ∂ðℇϝℱℊℋℌℍℎℏıℑȷℒℓℕ℘ℙℚℛℜℝℤ℧Åℬℭℰℯℳℴℵℶℷℸⅅⅆⅇⅈⅉ"),
    ("Operators", "+−÷×±∓∝∕∗∘∙√∛∜∩∪⊎⊓⊔∧∨⊕⊖⊗⊘⊙⊚⊛⊞⊟⊠⊡⋄⋅⋆⋉⋊∔∸∖≀=≠<>≤≥≮≰≯≱≡∼≃≈≅≢≄≉≇∝≪≫∈∋∉⊂⊃⊆⊇≺≻≼≽⊏⊐⊑⊒∥⊥⊢⊣⋈≍∴∵∷≐≑≒≓≔≕≖≗≜≝≞≟∑∫∬∭∮∯∰∏∐⋂⋃⋀⋁⨀⨁⨂⨄⨆"),
    ("Arrows", "←→↑↓↔↕⇐⇒⇑⇓⇔⇕⟵⟶⟷⟸⟹⟺↦⟼↩↪↼↽⇀⇁↿↾⇃⇂⇋⇌⇄⇆⇇⇉⇈⇊↖↗↘↙↶↷↺↻⊸↜↝↞↠↢↣↫↬↭⇚⇛↰↱⇝"),
    ("Negated Relations", "≠≮≯≰≱≢≁≄≉≇≭≨≩⊀⊁⋠⋡∉∌⊄⊅⊈⊉⊊⊋⋢⋣⋦⋧⋨⋩⋪⋫⋬⋭∤∦⊬⊭⊮⊯∄"),
    ("Scripts", "𝒜ℬ𝒞𝒟ℰℱ𝒢ℋℐ𝒥𝒦ℒℳ𝒩𝒪𝒫𝒬ℛ𝒮𝒯𝒰𝒱𝒲𝒳𝒴𝒵𝒶𝒷𝒸𝒹ℯ𝒻ℊ𝒽𝒾𝒿𝓀𝓁𝓂𝓃ℴ𝓅𝓆𝓇𝓈𝓉𝓊𝓋𝓌𝓍𝓎𝓏𝔄𝔅ℭ𝔇𝔈𝔉𝔊ℌℑ𝔍𝔎𝔏𝔐𝔑𝔒𝔓𝔔ℜ𝔖𝔗𝔘𝔙𝔚𝔛𝔜ℨ𝔸𝔹ℂ𝔻𝔼𝔽𝔾ℍ𝕀𝕁𝕂𝕃𝕄ℕ𝕆ℙℚℝ𝕊𝕋𝕌𝕍𝕎𝕏𝕐ℤ"),
    ("Geometry", "∟∠∡∢⊾⊿⋕⊥∤∥∦∶∷∴∵∎△▭▱○◊□◇⊡⊙"),
];

/// Ready-made equations (Insert › Equation), grouped by subject: geometry, algebra, calculus,
/// trigonometry, probability. The labels are our own descriptions of the formulas.
pub const BUILT_INS: &[(&str, &str, &str)] = &[
    ("pythagoras", "Sides of a right triangle", "a^2+b^2=c^2"),
    ("area", "Circle area from its radius", "A=πr^2"),
    ("quadratic", "Roots of ax²+bx+c", "x=(−b±√(b^2−4ac))/2a"),
    ("binomial", "Binomial expansion of (x+a)ⁿ", "(x+a)^n=∑_(k=0)^n▒〖(n¦k) x^k a^(n−k)〗"),
    ("expansion", "Power series of (1+x)ⁿ", "(1+x)^n=1+nx/1!+(n(n−1)x^2)/2!+⋯"),
    ("euler", "Euler's identity", "e^(iπ)+1=0"),
    ("derivative", "Derivative as a limit", "f'(x)=lim┬(h→0)⁡〖(f(x+h)−f(x))/h〗"),
    ("taylor", "Exponential series", "e^x=1+x/1!+x^2/2!+x^3/3!+⋯,  −∞<x<∞"),
    ("gaussian", "Integral of e^(−x²)", "∫_(−∞)^∞▒e^(−x^2) dx=√π"),
    ("fourier", "Sine and cosine series of f(x)", "f(x)=a_0+∑_(n=1)^∞▒〖(a_n cos⁡〖nπx/L〗+b_n sin⁡〖nπx/L〗)〗"),
    ("trig1", "Sum or difference of sines", "sin⁡α±sin⁡β=2 sin⁡〖1/2(α±β)〗 cos⁡〖1/2(α∓β)〗"),
    ("trig2", "Sum of cosines", "cos⁡α+cos⁡β=2 cos⁡〖1/2(α+β)〗 cos⁡〖1/2(α−β)〗"),
    ("normal", "Normal probability density", "f(x)=1/(σ√(2π)) e^(−(x−μ)^2/(2σ^2))"),
    ("bayes", "Bayes' rule", "P(A|B)=(P(B|A)P(A))/P(B)"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::{MNode, to_linear};

    fn has_leftover(a: &[MNode]) -> bool {
        a.iter().any(|n| match n {
            MNode::Run(r) => r.text.contains('⬚') || r.text.contains('▒') || r.text.contains('┬') || r.text.contains('┴'),
            other => {
                (0..wordcraft_doc::math_edit::child_count(other)).any(|c| wordcraft_doc::math_edit::child(other, c).is_some_and(|x| has_leftover(x)))
            }
        })
    }

    /// A script or above/below operand takes one letter unless it is grouped (`a_(mn)`), as in
    /// Word; letters right after one mean a template split a word (`⟶┴yields` put only `y` above
    /// the arrow).
    fn split_operand(a: &[MNode]) -> bool {
        a.windows(2).any(|w| {
            matches!(w[0], MNode::Lim { .. } | MNode::Script { .. })
                && matches!(&w[1], MNode::Run(r) if r.text.starts_with(|c: char| c.is_alphabetic()))
        }) || a
            .iter()
            .any(|n| (0..wordcraft_doc::math_edit::child_count(n)).any(|c| wordcraft_doc::math_edit::child(n, c).is_some_and(|x| split_operand(x))))
    }

    #[test]
    fn multi_letter_operands_are_grouped() {
        for tpl in templates() {
            assert!(!split_operand(&tpl.nodes()), "{} splits an operand: {}", tpl.id, tpl.linear);
        }
        for (id, _, lin) in BUILT_INS {
            assert!(!split_operand(&parse_linear(lin)), "{id} splits an operand: {lin}");
        }
    }

    #[test]
    fn every_template_builds() {
        let mut ids = std::collections::HashSet::new();
        for tpl in templates() {
            assert!(ids.insert(tpl.id), "duplicate id {}", tpl.id);
            let n = tpl.nodes();
            assert!(!n.is_empty(), "{} builds nothing", tpl.id);
            assert!(!has_leftover(&n), "{} left linear operators: {}", tpl.id, to_linear(&n));
            // Templates with placeholders are structures, not text.
            if tpl.linear.contains('⬚') {
                assert!(n.iter().any(|x| !matches!(x, MNode::Run(_))), "{} → {n:?}", tpl.id);
            }
        }
        for (id, _, lin) in BUILT_INS {
            let n = parse_linear(lin);
            assert!(!has_leftover(&n), "{id}: {}", to_linear(&n));
        }
    }
}
