//! A themed line chart for data at evenly spaced intervals.
use crate::{theme, zoom::px};
use bruhs_models::num::ToFloatLossy as _;
use gpui::{prelude::*, *};

/// Labels for one interval on the horizontal axis and in its tooltip.
pub struct LineChartInterval {
    pub axis_label: SharedString,
    pub tooltip_title: SharedString,
}

/// One sample. Missing values leave a gap, and the tooltip shows the supplied text.
pub struct LineChartPoint {
    pub value: Option<f64>,
    pub tooltip_value: SharedString,
}

/// A named series, indexed in the same order as the chart intervals.
pub struct LineChartSeries {
    pub label: SharedString,
    pub color: Hsla,
    pub points: Vec<LineChartPoint>,
}

/// Optional presentation for the plot. Straight lines with no fill are the default.
#[derive(Clone, Copy, Default)]
pub struct LineChartStyle {
    /// Smooth each interval without overshooting either sample.
    pub rounded: bool,
    /// Opacity of the series-colored area above zero, from 0 to 1.
    pub fill_opacity: f32,
}

/// A color key matching the chart stroke.
pub fn line_swatch(color: Hsla) -> Div {
    div()
        .w(px(10.))
        .h(px(2.))
        .rounded_full()
        .flex_none()
        .bg(color)
}

/// Plot nonnegative values against a zero baseline. Supply a formatter for the vertical axis.
pub fn line_chart(
    id: &'static str,
    accessible_label: impl Into<SharedString>,
    height: Pixels,
    intervals: Vec<LineChartInterval>,
    series: Vec<LineChartSeries>,
    style: LineChartStyle,
    format_axis: impl Fn(f64) -> String,
) -> Div {
    let count = intervals.len();
    let maximum = series
        .iter()
        .flat_map(|series| {
            series
                .points
                .iter()
                .take(count)
                .filter_map(|point| point.value)
        })
        .filter(|value| value.is_finite())
        .fold(0., f64::max);
    let axis_text = |label: SharedString| {
        div()
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(theme::rambutan_400())
            .child(label)
    };
    let axis = div()
        .flex()
        .justify_between()
        .children(
            intervals
                .first()
                .map(|interval| axis_text(interval.axis_label.clone())),
        )
        .when(count > 2, |el| {
            el.child(axis_text(intervals[count / 2].axis_label.clone()))
        })
        .when(count > 1, |el| {
            el.children(
                intervals
                    .last()
                    .map(|interval| axis_text(interval.axis_label.clone())),
            )
        });
    let hover_targets: Vec<_> = intervals
        .into_iter()
        .enumerate()
        .map(|(index, interval)| {
            let rows: Vec<_> = series
                .iter()
                .filter_map(|series| {
                    let sample = series.points.get(index)?;
                    Some((
                        series.color,
                        series.label.clone(),
                        sample.tooltip_value.clone(),
                    ))
                })
                .collect();
            div()
                .id((id, index))
                .flex_1()
                .min_w_0()
                .hover(|el| el.bg(theme::rambutan_400().opacity(0.13)))
                .tooltip(move |_, cx| {
                    cx.new(|_| IntervalTooltip {
                        heading: interval.tooltip_title.clone(),
                        rows: rows.clone(),
                    })
                    .into()
                })
        })
        .collect();
    let plot = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Image)
        .aria_label(accessible_label)
        .relative()
        .h(height)
        .flex()
        .border_b_1()
        .border_color(theme::longan_700())
        .child(plot_lines(series, count, maximum, style))
        .children(hover_targets);
    div()
        .flex()
        .gap_2()
        .child(
            div()
                .h(height)
                .flex()
                .flex_col()
                .justify_between()
                .items_end()
                .when(maximum > 0., |el| {
                    el.child(axis_text(format_axis(maximum).into()))
                        .child(axis_text(format_axis(maximum / 2.).into()))
                })
                .child(axis_text(format_axis(0.).into())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(plot)
                .child(axis),
        )
}

fn plot_lines(
    series: Vec<LineChartSeries>,
    count: usize,
    maximum: f64,
    style: LineChartStyle,
) -> impl IntoElement {
    let grid = theme::longan_700();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let plot_height = (bounds.size.height - px(4.)).max(px(0.));
            let top = bounds.top() + px(4.);
            let baseline = top + plot_height;
            for level in [0., 0.5] {
                let y = top + plot_height * level;
                window.paint_quad(fill(
                    Bounds::new(point(bounds.left(), y), size(bounds.size.width, px(1.))),
                    grid,
                ));
            }
            if maximum <= 0. || count == 0 {
                return;
            }
            let step = bounds.size.width / count.to_f32_lossy();
            let projected: Vec<_> = series
                .iter()
                .map(|series| {
                    let points: Vec<_> = series
                        .points
                        .iter()
                        .take(count)
                        .enumerate()
                        .map(|(index, sample)| {
                            let value = sample
                                .value
                                .filter(|value| value.is_finite() && *value >= 0.)?;
                            Some(point(
                                bounds.left() + step * (index.to_f32_lossy() + 0.5),
                                top + plot_height * (1. - (value / maximum).to_f32_lossy()),
                            ))
                        })
                        .collect();
                    (series.color, points)
                })
                .collect();

            if style.fill_opacity > 0. {
                for (color, points) in &projected {
                    let color = color.opacity(style.fill_opacity.min(1.));
                    for run in points.split(Option::is_none).filter(|run| run.len() >= 2) {
                        let mut path = PathBuilder::fill();
                        if let Some((first, last)) = trace_run(&mut path, run, style.rounded) {
                            path.line_to(point(last.x, baseline));
                            path.line_to(point(first.x, baseline));
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color);
                            }
                        }
                    }
                }
            }

            for (color, points) in &projected {
                let mut path = PathBuilder::stroke(px(1.5));
                for run in points.split(Option::is_none) {
                    let _ = trace_run(&mut path, run, style.rounded);
                    if !style.rounded || run.len() == 1 {
                        for &position in run
                            .iter()
                            .flatten()
                            .filter(|position| position.y < baseline)
                        {
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        position - point(px(1.5), px(1.5)),
                                        size(px(3.), px(3.)),
                                    ),
                                    *color,
                                )
                                .corner_radii(px(1.5)),
                            );
                        }
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, *color);
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

fn trace_run(
    path: &mut PathBuilder,
    run: &[Option<Point<Pixels>>],
    rounded: bool,
) -> Option<(Point<Pixels>, Point<Pixels>)> {
    let mut points = run.iter().flatten().copied();
    let first = points.next()?;
    path.move_to(first);
    let mut previous = first;
    for next in points {
        if rounded {
            let mid_x = previous.x + (next.x - previous.x) / 2.;
            path.cubic_bezier_to(next, point(mid_x, previous.y), point(mid_x, next.y));
        } else {
            path.line_to(next);
        }
        previous = next;
    }
    Some((first, previous))
}

struct IntervalTooltip {
    heading: SharedString,
    rows: Vec<(Hsla, SharedString, SharedString)>,
}

impl Render for IntervalTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().p_3().child(
            div()
                .id("line-chart-tooltip")
                .role(Role::Tooltip)
                .rounded(px(8.))
                .border_1()
                .border_color(theme::ring_floating())
                .bg(theme::longan_700())
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .text_size(px(12.))
                .font_family(theme::FontFace::Small)
                .text_color(theme::rambutan_100())
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.heading.clone()),
                )
                .children(self.rows.iter().map(|(color, label, amount)| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(line_swatch(*color))
                                .child(label.clone()),
                        )
                        .child(amount.clone())
                })),
        )
    }
}
