use std::cell::RefCell;
use std::ffi::c_int;
use std::process::id;
use std::rc::Rc;
use glfw::{Action, Key, MouseButton, WindowEvent};
use crate::gs::nvg;
use crate::gs::nvg::{Color, Image, NvgContext, Point, Shape, Text, TextHorizontalAlignment, TextVerticalAlignment};
use crate::gs::window::Window;
use crate::player::surface::VideoSurface;

pub type ComponentId = usize;
pub type GroupWeight = f32;

pub enum InputEvent {
    MouseMoved((f32, f32), (f32, f32)),
    MouseButton(MouseButton, Action, (f32, f32)),
    Key(Key, Action)
}

pub enum ComponentBody {
    Empty,
    Root {
        children: Vec<ComponentId>,
    },
    Label {
        text: Text,
        color: Color,
        alignment: (TextHorizontalAlignment, TextVerticalAlignment),
    },
    Rect {
        color: Color,
    },
    Button {
        text: Text,
        pressed: bool,
    },
    VideoSurface {
        surface: Rc<RefCell<VideoSurface>>,
        image: Option<Image>
    },
    TextInput {
        text: Text,
        begin_index: Option<usize>,
        end_index: Option<usize>,
        enter_pressed: bool,
    },
    Slider {
        percent: f32,
        target: Option<f32>,
        foreground: Color,
        background: Color,
    },
    VerticalGroup {
        children: Vec<(GroupWeight, Option<ComponentId>)>
    },
    HorizontalGroup {
        children: Vec<(GroupWeight, Option<ComponentId>)>
    },
    Pane {
        children: Vec<ComponentId>,
    }
}

pub struct Component {
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
    pub preferred_width: Option<f32>,
    pub preferred_height: Option<f32>,
    pub padding: f32,
    pub visible: bool,
    pub body: ComponentBody,
}

impl Component {

    pub fn new() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            x: 0.0,
            y: 0.0,
            padding: 0.0,
            preferred_width: None,
            preferred_height: None,
            visible: true,
            body: ComponentBody::Empty,
        }
    }

    pub fn with_preferred_size(mut self, width: Option<f32>, height: Option<f32>) -> Self {
        self.preferred_width = width;
        self.preferred_height = height;
        self
    }

    pub fn with_padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn hgroup(children: Vec<(GroupWeight, Option<ComponentId>)>) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::HorizontalGroup { children };
        comp
    }

    pub fn vgroup(children: Vec<(GroupWeight, Option<ComponentId>)>) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::VerticalGroup { children };
        comp
    }

    pub fn slider(foreground: Color, background: Color) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::Slider { percent: 0.0, target: None, foreground, background };
        comp
    }

    pub fn video_surface(surface: Rc<RefCell<VideoSurface>>) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::VideoSurface { surface, image: None };
        comp
    }

    pub fn pane(children: Vec<ComponentId>) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::Pane { children };
        comp
    }

    pub fn root(children: Vec<ComponentId>) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::Root { children };
        comp
    }

    pub fn label(nvg: &mut NvgContext, color: Color, alignment: (TextHorizontalAlignment, TextVerticalAlignment)) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::Label {
            text: nvg.text("", "default", 13.0), 
            color, alignment
        };
        comp
    }

    pub fn rect(color: Color) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::Rect { color };
        comp
    }

    pub fn bounds_rect(&self) -> Shape {
        let w = self.preferred_width.unwrap_or(self.width);
        let h = self.preferred_height.unwrap_or(self.height);
        Shape::Rect(self.x, self.y, w, h).with_padding(self.padding, true)
    }

    pub fn update(&mut self, manager: &mut ComponentManager, context: &mut nvg::NvgContext) {

    }

    pub fn handle_event(&mut self, event: InputEvent) {
        let (x0, y, w, h) = self.bounds_rect().bounds();
        match &mut self.body {
            ComponentBody::Button { text, pressed } => {
                if let InputEvent::MouseButton(MouseButton::Button1, Action::Press | Action::Repeat, _) = event {
                    *pressed = true;
                }
            }
            ComponentBody::Slider { percent, target, .. } => {
                if let InputEvent::MouseButton(MouseButton::Button1, Action::Release, (x, y)) = event {
                    *target = Some((x - x0) / w);
                }
            }
            _ => {}
        }
    }

    pub fn add_children(&mut self, id: Option<ComponentId>, weight: GroupWeight) {
        match &mut self.body {
            ComponentBody::HorizontalGroup { children } | ComponentBody::VerticalGroup { children } => {
                children.push((weight, id));
            }
            _ => panic!("Invalid operation!")
        }
    }

}

pub struct ComponentManager {
    components: Vec<Option<Component>>,
    free: Vec<ComponentId>,
    focused: Option<ComponentId>,
    root: Option<ComponentId>,

    mouse_pos: (f32, f32),
}

impl ComponentManager {
    pub fn new() -> ComponentManager {
        ComponentManager {
            components: Vec::new(),
            free: Vec::new(),
            focused: None,
            root: None,
            mouse_pos: (0.0, 0.0),
        }
    }

    pub fn intersecting_child(&self, x: f32, y: f32, parent: Option<ComponentId>) -> Option<ComponentId> {
        let child = if let Some(parent) = parent.or(self.root).and_then(|id| self.get(id)) {
            match &parent.body {
                ComponentBody::HorizontalGroup { children } | ComponentBody::VerticalGroup { children } => {
                    children.iter().find(|(_, id)| {
                        if let Some(child) = id.and_then(|id| self.get(id)) {
                            if child.bounds_rect().intersects(Point::new(x, y)) {
                                return true;
                            }
                        }
                        false
                    }).map(|(_, id)| id.unwrap())
                }
                ComponentBody::Pane { children } | ComponentBody::Root { children } => {
                    children.iter().rev().find(|id| {
                        if let Some(child) = self.get(**id) {
                            if child.bounds_rect().intersects(Point::new(x, y)) {
                                return true;
                            }
                        }
                        false
                    }).map(|id| *id)
                }
                _ => None
            }
        } else {
            None
        };

        if child.is_none() {
            return None
        }

        if let Some(id) = self.intersecting_child(x, y, child) {
            Some(id)
        } else {
            child
        }
    }

    pub fn remove(&mut self, id: ComponentId) -> Option<Component> {
        let option = self.components.get_mut(id);
        if option.is_none() {
            return None;
        }
        let component = option.unwrap();
        if component.is_some() {
            self.free.push(id);
            return component.take();
        }
        None
    }

    pub fn new_pane(&mut self, children: Vec<ComponentId>) -> ComponentId {
        self.push(Component::pane(children))
    }

    pub fn push(&mut self, component: Component) -> ComponentId {
        if let Some(id) = self.free.pop() {
            self.components[id] = Some(component);
            id
        } else {
            self.components.push(Some(component));
            self.components.len() - 1
        }
    }

    pub fn get_mut_body(&mut self, id: ComponentId) -> Option<&mut ComponentBody> {
        self.components.get_mut(id).and_then(Option::as_mut).map(|c| &mut c.body)
    }

    pub fn get_mut(&mut self, id: ComponentId) -> Option<&mut Component> {
        self.components.get_mut(id).and_then(Option::as_mut)
    }

    pub fn get(&self, id: ComponentId) -> Option<&Component> {
        self.components.get(id).and_then(Option::as_ref)
    }

    pub fn render(&mut self, id: ComponentId, renderer: &mut nvg::NvgContext) {
        let mut comp = match self.components[id].take() {
            Some(comp) => comp,
            None => return
        };
        if !comp.visible {
            return;
        }
        let rect = comp.bounds_rect();
        let (mut x, mut y, w, h) = rect.bounds();
        match &mut comp.body {
            ComponentBody::Empty => {}
            ComponentBody::Root { children } => {
                for child in children {
                    if let Some(child) = self.get_mut(*child) {
                        child.x = x;
                        child.y = y;
                        child.width = child.preferred_width.unwrap_or(w);
                        child.height = child.preferred_height.unwrap_or(h);
                    }
                    self.render(*child, renderer);
                }
            },
            ComponentBody::Pane { children } => {
                for child in children {
                    self.render(*child, renderer);
                }
            }
            ComponentBody::Label { text, color, alignment: (horizontal, vertial) } => {
                renderer.begin_path();
                renderer.fit_text(text, comp.width, comp.height);
                renderer.fill_color(*color);
                renderer.draw_text_inside(text, rect, *horizontal, *vertial);
            }
            ComponentBody::Rect { color } => {
                renderer.begin_path();
                renderer.draw_shape(Shape::Rect(x, y, w, h));
                renderer.fill_color(*color);
                renderer.fill();
            }
            ComponentBody::Slider { percent, target, foreground, background } => {
                renderer.begin_path();
                renderer.draw_shape(Shape::Rect(x, y, w, h));
                renderer.fill_color(*background);
                renderer.fill();

                let w = w * target.unwrap_or(*percent);
                renderer.begin_path();
                renderer.draw_shape(Shape::Rect(x, y, w, h));
                renderer.fill_color(*foreground);
                renderer.fill();
            }

            ComponentBody::VideoSurface { surface, image } => {
                let mut surface = surface.borrow_mut();
                surface.update();
                if let Some((_, _)) = surface.size_update.take() {
                    if let Some(image) = image.take() {
                        renderer.delete_image(image);
                    }
                    *image = Some(renderer.create_texture_image(&surface.output_texture));
                }
                if let Some(image) = image {
                    renderer.begin_path();
                    let (pw, ph) = image.size_conserve_aspect_ratio(w, h);
                    let ox = (w - pw) / 2.0;
                    let oy = (h - ph) / 2.0;
                    let rect = Shape::Rect(x + ox, y + oy, pw, ph);
                    let paint = renderer.image_paint(image, rect, 1.0);
                    renderer.draw_shape(rect);
                    renderer.fill_paint(paint);
                    renderer.fill();
                }
            }

            ComponentBody::VerticalGroup { children } => {
                let sum = children.iter().map(|(w, _)| w ).sum::<f32>();
                for (weight, id) in children {

                    let h = h * (*weight / sum);
                    if let Some(id) = id {
                        let comp = self.get_mut(*id);
                        if let Some(comp) = comp {
                            comp.x = x;
                            comp.y = y;
                            comp.width = comp.preferred_width.unwrap_or(w);
                            comp.height = comp.preferred_height.unwrap_or(h);
                        }
                        self.render(*id, renderer);
                    }
                    y += h;
                }
            }
            ComponentBody::HorizontalGroup { children } => {
                let sum = children.iter().map(|(w, _)| w ).sum::<f32>();
                for (weight, id) in children {
                    let w = w * (*weight / sum);
                    if let Some(id) = id {
                        let comp = self.get_mut(*id);
                        if let Some(comp) = comp {
                            comp.x = x;
                            comp.y = y;
                            comp.width = comp.preferred_width.unwrap_or(w);
                            comp.height = comp.preferred_height.unwrap_or(h);
                        }
                        self.render(*id, renderer);
                    }
                    x += w;
                }
            }
            _ => unimplemented!()
        }
        self.components[id] = Some(comp);
    }

    pub fn set_root(&mut self, root: ComponentId) {
        self.root = Some(root);
    }

    pub fn render_root(&mut self, renderer: &mut nvg::NvgContext) {
        if let Some(root) = self.root {
            if let Some(root) = self.get_mut(root) {
                root.x = 0.0;
                root.y = 0.0;
                root.width = renderer.width(None);
                root.height = renderer.height(None);
            }
            self.render(root, renderer);
        }
    }

    pub fn handle_event(&mut self, event: WindowEvent, window: &Window) {
        match event {
            WindowEvent::Key(key, _, action, _) => {
                if let Some(focused) = self.focused.and_then(|id| self.get_mut(id)) {
                    focused.handle_event(InputEvent::Key(key, action));
                }
            }
            WindowEvent::CursorPos(x, y) => {
                let y = window.get_size().1 as f64 - y;
                let (lx, ly) = self.mouse_pos;
                self.mouse_pos = (x as f32, y as f32);
                if let Some(child) = self.intersecting_child(x as f32, y as f32, None)
                    .and_then(|id| self.get_mut(id)) {
                    child.handle_event(InputEvent::MouseMoved((lx, ly), (x as f32, y as f32)));
                }
            }
            WindowEvent::MouseButton(button, action, _) => {
                let (x, y) = self.mouse_pos;
                if let Some((id, child)) = self.intersecting_child(x, y, None)
                    .and_then(|id| self.get_mut(id).map(|comp| (id, comp))) {
                    child.handle_event(InputEvent::MouseButton(button, action, (x, y)));
                    self.focused = Some(id);
                }
            }
            _ => {}
        }
    }
}