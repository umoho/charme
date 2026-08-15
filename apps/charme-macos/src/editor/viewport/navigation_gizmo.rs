use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    f64::consts::{FRAC_PI_2, PI, TAU},
    sync::OnceLock,
};

use cacao::{
    appkit::App,
    color::Color,
    foundation::{NO, NSArray, YES, id, nil},
    image::{DrawConfig, Image, ImageView, ResizeBehavior},
    layout::Layout,
    objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    },
    text::{Label, TextAlign},
    utils::properties::ObjcProperty,
    view::View,
};
use core_graphics::{
    context::{CGContextRef, CGLineCap},
    geometry::{CGPoint, CGRect, CGSize},
};

use crate::{
    app::{CharmeApp, Message},
    ui::label,
};

const GIZMO_SIZE: f64 = 128.0;
const GIZMO_CENTER: f64 = GIZMO_SIZE * 0.5;
const AXIS_RADIUS: f64 = 37.0;
const ENDPOINT_BASE_RADIUS: f64 = 4.0;
const ENDPOINT_RADIUS_VARIATION: f64 = 1.2;
const NEGATIVE_ENDPOINT_BASE_RADIUS: f64 = 5.5;
const NEGATIVE_ENDPOINT_RADIUS_VARIATION: f64 = 1.2;
const LABEL_SIZE: f64 = 20.0;
const LABEL_INSET: f64 = 2.0;
const PITCH_LIMIT: f32 = 1.45;

#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraOrientation {
    yaw: f32,
    pitch: f32,
}

impl Default for CameraOrientation {
    fn default() -> Self {
        Self {
            yaw: -0.55,
            pitch: -0.35,
        }
    }
}

#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct ProjectedAxis {
    index: usize,
    positive: Point,
    negative: Point,
    depth: f64,
}

#[derive(Clone, Copy)]
struct EndpointStyle {
    radius: f64,
    alpha: f64,
}

#[derive(Clone, Copy)]
struct EndpointMarker {
    index: usize,
    point: Point,
    depth: f64,
    style: EndpointStyle,
    filled: bool,
}

pub(crate) struct NavigationGizmo {
    pub(crate) view: View,
    image_view: ImageView,
    x_label: Label,
    y_label: Label,
    z_label: Label,
    _input: ObjcProperty,
    orientation: Cell<CameraOrientation>,
    current_image: RefCell<Option<Image>>,
}

impl NavigationGizmo {
    pub(crate) fn new() -> Self {
        let view = View::new();
        let image_view = ImageView::new();
        image_view.set_background_color(Color::Clear);

        let x_label = axis_label("X", Color::rgb(238, 55, 75));
        let y_label = axis_label("Y", Color::rgb(118, 194, 39));
        let z_label = axis_label("Z", Color::rgb(48, 132, 224));
        view.add_subview(&image_view);
        for label in [&x_label, &y_label, &z_label] {
            view.add_subview(label);
        }

        let input = unsafe {
            let input: id = msg_send![input_class(), new];
            let _: () = msg_send![input, setTranslatesAutoresizingMaskIntoConstraints: NO];
            input
        };
        view.objc.with_mut(|container| unsafe {
            let _: () = msg_send![container, addSubview: input];
            pin_to_edges(input, container);
        });

        let orientation = CameraOrientation::default();
        let image = draw_gizmo(orientation);
        image_view.set_image(&image);

        let gizmo = Self {
            view,
            image_view,
            x_label,
            y_label,
            z_label,
            _input: ObjcProperty::retain(input),
            orientation: Cell::new(orientation),
            current_image: RefCell::new(Some(image)),
        };
        gizmo.layout_labels(orientation);
        gizmo
    }

    pub(crate) fn orbit(&self, delta_yaw: f32, delta_pitch: f32) {
        let mut orientation = self.orientation.get();
        orientation.yaw += delta_yaw;
        orientation.pitch = (orientation.pitch + delta_pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.set_orientation(orientation);
    }

    pub(crate) fn reset(&self) {
        self.set_orientation(CameraOrientation::default());
    }

    fn set_orientation(&self, orientation: CameraOrientation) {
        self.orientation.set(orientation);
        let image = draw_gizmo(orientation);
        self.image_view.set_image(&image);
        self.current_image.replace(Some(image));
        self.layout_labels(orientation);
    }

    pub(crate) fn orbit_delta_at(&self, x: f64, y: f64) -> Option<(f32, f32)> {
        let target = self.target_orientation_at(x, y)?;
        let current = self.orientation.get();
        Some((
            shortest_angle_delta(target.yaw, current.yaw),
            target.pitch - current.pitch,
        ))
    }

    fn target_orientation_at(&self, x: f64, y: f64) -> Option<CameraOrientation> {
        let projected = projected_axes(self.orientation.get());
        let point = Point { x, y };
        let mut best: Option<(f64, usize, bool)> = None;

        for axis in projected {
            for (endpoint, positive) in [(axis.positive, true), (axis.negative, false)] {
                let distance = squared_distance(point, endpoint);
                let depth = if positive { axis.depth } else { -axis.depth };
                let hit_radius = if positive {
                    LABEL_SIZE * 0.65
                } else {
                    negative_endpoint_style(depth).radius * 1.8
                };
                if distance > hit_radius * hit_radius {
                    continue;
                }
                if best.is_none_or(|(best_distance, _, _)| distance < best_distance) {
                    best = Some((distance, axis.index, positive));
                }
            }
        }

        best.map(|(_, index, positive)| axis_orientation(index, positive))
    }

    fn layout_labels(&self, orientation: CameraOrientation) {
        for (label, axis) in [
            (&self.x_label, 0usize),
            (&self.y_label, 1usize),
            (&self.z_label, 2usize),
        ] {
            let projected = projected_axes(orientation)[axis];
            let endpoint = projected.positive;
            let style = endpoint_style(projected.depth);
            let (red, green, blue) = axis_color(axis);
            label.set_text_color(Color::rgba(
                (red * 255.0) as u8,
                (green * 255.0) as u8,
                (blue * 255.0) as u8,
                (style.alpha * 255.0) as u8,
            ));
            let x = (endpoint.x - LABEL_SIZE * 0.5)
                .clamp(LABEL_INSET, GIZMO_SIZE - LABEL_SIZE - LABEL_INSET);
            let y = (endpoint.y - LABEL_SIZE * 0.5)
                .clamp(LABEL_INSET, GIZMO_SIZE - LABEL_SIZE - LABEL_INSET);
            label.set_frame(CGRect::new(
                &CGPoint::new(x, y),
                &CGSize::new(LABEL_SIZE, LABEL_SIZE),
            ));
        }
    }
}

fn axis_label(text: &str, color: Color) -> Label {
    let label = label(text, 12.0, true, color);
    label.set_text_alignment(TextAlign::Center);
    label.set_background_color(Color::Clear);
    label.set_translates_autoresizing_mask_into_constraints(true);
    label
}

fn draw_gizmo(orientation: CameraOrientation) -> Image {
    let config = DrawConfig {
        source: (GIZMO_SIZE, GIZMO_SIZE),
        target: (GIZMO_SIZE, GIZMO_SIZE),
        resize: ResizeBehavior::Stretch,
    };
    Image::draw(config, move |_, context| {
        draw_axes(context, orientation);
        true
    })
}

fn draw_axes(context: &CGContextRef, orientation: CameraOrientation) {
    let axes = projected_axes(orientation);

    context.set_line_cap(CGLineCap::CGLineCapRound);
    context.set_line_width(2.5);

    // Draw the spokes first. Endpoint markers are drawn separately below so
    // that the six individual endpoints can be depth-sorted.
    for axis in axes {
        let (red, green, blue) = axis_color(axis.index);
        let positive_style = endpoint_style(axis.depth);
        context.set_rgb_stroke_color(red, green, blue, positive_style.alpha);
        context.begin_path();
        context.move_to_point(GIZMO_CENTER, GIZMO_CENTER);
        context.add_line_to_point(axis.positive.x, axis.positive.y);
        context.stroke_path();
    }

    let mut endpoints = [
        endpoint_marker(axes[0], true),
        endpoint_marker(axes[0], false),
        endpoint_marker(axes[1], true),
        endpoint_marker(axes[1], false),
        endpoint_marker(axes[2], true),
        endpoint_marker(axes[2], false),
    ];
    endpoints.sort_by(|left, right| {
        left.depth
            .partial_cmp(&right.depth)
            .unwrap_or(Ordering::Equal)
    });

    for endpoint in endpoints {
        let (red, green, blue) = axis_color(endpoint.index);
        if endpoint.filled {
            context.set_rgb_fill_color(red, green, blue, endpoint.style.alpha);
            context.fill_ellipse_in_rect(circle_rect(endpoint.point, endpoint.style.radius));
        } else {
            context.set_rgb_stroke_color(red, green, blue, endpoint.style.alpha);
            context.stroke_ellipse_in_rect(circle_rect(endpoint.point, endpoint.style.radius));
        }
    }

    context.set_rgb_fill_color(0.08, 0.09, 0.11, 0.92);
    context.fill_ellipse_in_rect(circle_rect(
        Point {
            x: GIZMO_CENTER,
            y: GIZMO_CENTER,
        },
        3.0,
    ));
}

fn projected_axes(orientation: CameraOrientation) -> [ProjectedAxis; 3] {
    [
        projected_axis(0, 1.0, 0.0, 0.0, orientation),
        projected_axis(1, 0.0, 1.0, 0.0, orientation),
        projected_axis(2, 0.0, 0.0, 1.0, orientation),
    ]
}

fn projected_axis(
    index: usize,
    world_x: f64,
    world_y: f64,
    world_z: f64,
    orientation: CameraOrientation,
) -> ProjectedAxis {
    let yaw = f64::from(orientation.yaw);
    let pitch = f64::from(orientation.pitch);
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();

    // This mirrors the renderer's YXZ orbit rotation: Ry(yaw) * Rx(pitch).
    let rotated_x = cos_yaw * world_x - sin_yaw * world_z;
    let rotated_z = sin_yaw * world_x + cos_yaw * world_z;
    let camera_x = rotated_x;
    let camera_y = cos_pitch * world_y + sin_pitch * rotated_z;
    let depth = -sin_pitch * world_y + cos_pitch * rotated_z;
    let endpoint = Point {
        x: GIZMO_CENTER + camera_x * AXIS_RADIUS,
        y: GIZMO_CENTER - camera_y * AXIS_RADIUS,
    };

    ProjectedAxis {
        index,
        positive: endpoint,
        negative: Point {
            x: GIZMO_CENTER - camera_x * AXIS_RADIUS,
            y: GIZMO_CENTER + camera_y * AXIS_RADIUS,
        },
        depth,
    }
}

fn axis_orientation(index: usize, positive: bool) -> CameraOrientation {
    match index {
        0 => CameraOrientation {
            yaw: if positive {
                FRAC_PI_2 as f32
            } else {
                -FRAC_PI_2 as f32
            },
            pitch: 0.0,
        },
        1 => CameraOrientation {
            yaw: 0.0,
            pitch: if positive {
                -(FRAC_PI_2 as f32 - 0.14)
            } else {
                FRAC_PI_2 as f32 - 0.14
            },
        },
        _ => CameraOrientation {
            yaw: if positive { 0.0 } else { PI as f32 },
            pitch: 0.0,
        },
    }
}

fn squared_distance(first: Point, second: Point) -> f64 {
    let dx = first.x - second.x;
    let dy = first.y - second.y;
    dx * dx + dy * dy
}

fn endpoint_marker(axis: ProjectedAxis, positive: bool) -> EndpointMarker {
    let depth = if positive { axis.depth } else { -axis.depth };
    EndpointMarker {
        index: axis.index,
        point: if positive {
            axis.positive
        } else {
            axis.negative
        },
        depth,
        style: if positive {
            endpoint_style(depth)
        } else {
            negative_endpoint_style(depth)
        },
        filled: positive,
    }
}

fn endpoint_style(depth: f64) -> EndpointStyle {
    // The orbit basis uses +Z from the target toward the camera, so a positive
    // camera-space depth is closer to the viewer. Keep the effect deliberately
    // restrained, like Blender's orientation gizmo.
    let frontness = depth.clamp(-1.0, 1.0);
    let normalized = (frontness + 1.0) * 0.5;
    EndpointStyle {
        radius: ENDPOINT_BASE_RADIUS + ENDPOINT_RADIUS_VARIATION * normalized,
        alpha: 0.28 + 0.64 * normalized,
    }
}

fn negative_endpoint_style(depth: f64) -> EndpointStyle {
    let frontness = depth.clamp(-1.0, 1.0);
    let normalized = (frontness + 1.0) * 0.5;
    EndpointStyle {
        radius: NEGATIVE_ENDPOINT_BASE_RADIUS + NEGATIVE_ENDPOINT_RADIUS_VARIATION * normalized,
        alpha: 0.20 + 0.52 * normalized,
    }
}

fn circle_rect(center: Point, radius: f64) -> CGRect {
    CGRect::new(
        &CGPoint::new(center.x - radius, center.y - radius),
        &CGSize::new(radius * 2.0, radius * 2.0),
    )
}

fn axis_color(index: usize) -> (f64, f64, f64) {
    match index {
        0 => (0.93, 0.22, 0.29),
        1 => (0.46, 0.76, 0.15),
        _ => (0.19, 0.52, 0.88),
    }
}

fn pin_to_edges(child: id, parent: id) {
    unsafe {
        let child_leading: id = msg_send![child, leadingAnchor];
        let parent_leading: id = msg_send![parent, leadingAnchor];
        let child_trailing: id = msg_send![child, trailingAnchor];
        let parent_trailing: id = msg_send![parent, trailingAnchor];
        let child_top: id = msg_send![child, topAnchor];
        let parent_top: id = msg_send![parent, topAnchor];
        let child_bottom: id = msg_send![child, bottomAnchor];
        let parent_bottom: id = msg_send![parent, bottomAnchor];
        let constraints = NSArray::new(&[
            msg_send![child_leading, constraintEqualToAnchor: parent_leading],
            msg_send![child_trailing, constraintEqualToAnchor: parent_trailing],
            msg_send![child_top, constraintEqualToAnchor: parent_top],
            msg_send![child_bottom, constraintEqualToAnchor: parent_bottom],
        ]);
        let _: () = msg_send![class!(NSLayoutConstraint), activateConstraints: &*constraints];
    }
}

fn input_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| unsafe {
        let mut declaration = ClassDecl::new("CharmeNavigationGizmoInput", class!(NSView))
            .expect("navigation gizmo input class should only be registered once");
        declaration.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(mouseDragged:),
            mouse_dragged as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> bool,
        );
        declaration.add_method(
            sel!(isFlipped),
            is_flipped as extern "C" fn(&Object, Sel) -> bool,
        );
        declaration.add_method(
            sel!(mouseDownCanMoveWindow),
            mouse_down_cannot_move_window as extern "C" fn(&Object, Sel) -> bool,
        );
        declaration.register()
    })
}

extern "C" fn mouse_down(view: &Object, _: Sel, event: id) {
    let window_point: CGPoint = unsafe { msg_send![event, locationInWindow] };
    let point: CGPoint = unsafe { msg_send![view, convertPoint: window_point fromView: nil] };
    App::<CharmeApp, Message>::dispatch_main(Message::NavigationGizmoMouseDown {
        x: point.x,
        y: point.y,
    });
}

extern "C" fn mouse_dragged(_: &Object, _: Sel, event: id) {
    let delta_x: f64 = unsafe { msg_send![event, deltaX] };
    let delta_y: f64 = unsafe { msg_send![event, deltaY] };
    App::<CharmeApp, Message>::dispatch_main(Message::Orbit {
        delta_x: -(delta_x as f32) * 0.01,
        delta_y: -(delta_y as f32) * 0.01,
    });
}

extern "C" fn accepts_first_mouse(_: &Object, _: Sel, _: id) -> bool {
    YES
}

extern "C" fn is_flipped(_: &Object, _: Sel) -> bool {
    YES
}

extern "C" fn mouse_down_cannot_move_window(_: &Object, _: Sel) -> bool {
    NO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_orientation_projects_three_distinct_axes() {
        let axes = projected_axes(CameraOrientation::default());
        assert!(axes[0].positive.x > GIZMO_CENTER);
        assert!(axes[1].positive.y < GIZMO_CENTER);
        assert_ne!(axes[2].positive.x, GIZMO_CENTER);
    }

    #[test]
    fn axis_targets_are_reachable_with_the_renderer_orbit_limits() {
        for index in 0..3 {
            for positive in [true, false] {
                let target = axis_orientation(index, positive);
                assert!(target.pitch.abs() <= PITCH_LIMIT);
            }
        }
    }

    #[test]
    fn angle_delta_uses_the_shortest_path() {
        assert!((shortest_angle_delta(PI as f32, -PI as f32 + 0.1) + 0.1).abs() < 0.001);
    }

    #[test]
    fn front_endpoints_are_subtly_larger_and_brighter() {
        let front = endpoint_style(1.0);
        let back = endpoint_style(-1.0);
        assert!(front.radius > back.radius);
        assert!(front.alpha > back.alpha);
        assert!(front.radius - back.radius < 2.0);
    }
}

fn shortest_angle_delta(target: f32, current: f32) -> f32 {
    let mut delta = (target - current) % TAU as f32;
    if delta > PI as f32 {
        delta -= TAU as f32;
    } else if delta < -(PI as f32) {
        delta += TAU as f32;
    }
    delta
}
