use crate::gs::nvg;
use crate::gs::nvg::{Color, Image, NvgContext, Point, Shape, Text, TextHorizontalAlignment, TextVerticalAlignment};
use crate::gs::window::Window;
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton, WindowEvent};
use mlua::prelude::LuaTable;
use mlua::{AnyUserData, AsChunk, FromLua, Function, IntoLua, Lua, Table, UserData, UserDataMethods, Value};
use std::cell::{Ref, RefCell};
use std::rc::Rc;
use json::value;
use crate::ffmpeg::input::Input;
use crate::player::player::VideoPlayer;

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
    Image(Image),
    ToggleButton {
        body: Option<ComponentId>,
        state: bool,
        pressed: Option<()>
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

pub enum PreferredSize {
    Auto,
    Fixed(f32),
    PercentParent(f32)
}

pub struct Style {
    pub preferred_size: (PreferredSize, PreferredSize),
    pub preferred_offset: (PreferredSize, PreferredSize),
    pub foreground_color: Option<Color>,
    pub background_color: Option<Color>,
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

    pub fn image(nvg: &mut NvgContext, path: &str) -> Option<Self> {
        nvg.create_image(path).map(|image| {
            let mut comp = Self::new();
            comp.body = ComponentBody::Image(image);
            comp
        })
    }

    pub fn toggle_button(body: ComponentId) -> Self {
        let mut comp = Self::new();
        comp.body = ComponentBody::ToggleButton { body: Some(body), state: false, pressed: None };
        comp
    }

    pub fn hgroup(mut children: Vec<(GroupWeight, Option<ComponentId>)>) -> Self {
        let mut comp = Self::new();
        let sum = children.iter().map(|(w, _)| w).sum::<f32>();
        children.iter_mut().for_each(|(w, _)| *w /= sum);
        comp.body = ComponentBody::HorizontalGroup { children };
        comp
    }

    pub fn vgroup(mut children: Vec<(GroupWeight, Option<ComponentId>)>) -> Self {
        let mut comp = Self::new();
        let sum = children.iter().map(|(w, _)| w).sum::<f32>();
        children.iter_mut().for_each(|(w, _)| *w /= sum);
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
            ComponentBody::ToggleButton { state, pressed, .. } => {
                if let InputEvent::MouseButton(MouseButton::Button1, Action::Release, _) = event {
                    *state = !*state;
                    *pressed = Some(());
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
                            if child.bounds_rect().intersects(Point::new(x, y)) && child.visible {
                                return true;
                            }
                        }
                        false
                    }).map(|(_, id)| id.unwrap())
                }
                ComponentBody::Pane { children } | ComponentBody::Root { children } => {
                    children.iter().rev().find(|id| {
                        if let Some(child) = self.get(**id) {
                            if child.bounds_rect().intersects(Point::new(x, y)) && child.visible {
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
        let mut comp = match self.components[id].take_if(|comp| comp.visible) {
            Some(comp) => comp,
            None => return
        };
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
                renderer.fit_text(text, w, h);
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
            ComponentBody::Image(image) => {
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
            ComponentBody::ToggleButton { body, .. } => {
                if let Some(body) = body.and_then(|id| self.get_mut(id)) {
                    body.x = x;
                    body.y = y;
                    body.width = w;
                    body.height = h;
                }
                if let Some(id) = body {
                    self.render(*id, renderer);
                }
            }

            ComponentBody::VideoSurface { surface, image } => {
                let mut surface = surface.borrow_mut();
                surface.update();
                if let Some((_, _)) = surface.size_update.take() {
                    if let Some(image) = image.take() {
                        renderer.delete_image(image);
                    }
                    *image = renderer.create_texture_image(&surface.output_texture);
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
                for (weight, id) in children {
                    let h = h * *weight;
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
                for (weight, id) in children {
                    let w = w * *weight;
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
                    if focused.visible {
                        focused.handle_event(InputEvent::Key(key, action));
                    }
                }
            }
            WindowEvent::CursorPos(x, y) => {
                let y = window.get_framebuffer_size().1 as f64 - y;
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

#[derive(Copy, Clone)]
pub enum RenderCommand {
    ShapeColor(Shape, Color),
}

impl UserData for VideoSurface {}
impl UserData for InputWorker {}
impl UserData for DecodeWorker {}
impl UserData for Text {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("draw", |_, this, (text): (String)| {
            this.clear();
            this.push_str(&text);
            Ok(())
        })
    }
}

struct UIRenderContext {
    list: Vec<RenderCommand>,
    nvg: Rc<RefCell<NvgContext>>
}

impl UIRenderContext {
    fn new(nvg: Rc<RefCell<NvgContext>>) -> Self {
        Self { list: Vec::new(), nvg }
    }
}

impl TryFrom<Table> for Color {
    type Error = mlua::Error;

    fn try_from(value: Table) -> Result<Self, Self::Error> {
        let r = value.get::<f32>(1)?;
        let g = value.get::<f32>(2)?;
        let b = value.get::<f32>(3)?;
        let a = value.get::<f32>(4)?;
        Ok(Color::rgba(r, g, b, a))
    }
}

impl TryFrom<Table> for Shape {
    type Error = mlua::Error;

    fn try_from(value: Table) -> Result<Self, Self::Error> {
        let type_ = value.get::<String>(1)?;
        match type_.as_str() {
            "rect" => {
                let x = value.get::<f32>(2)?;
                let y = value.get::<f32>(3)?;
                let width = value.get::<f32>(4)?;
                let height = value.get::<f32>(5)?;
                Ok(Shape::Rect(x, y, width, height))
            }
            _ => Err(Self::Error::RuntimeError(format!("Unknown shape: {}", type_))),
        }
    }
}

impl TryFrom<Table> for RenderCommand {
    type Error = mlua::Error;

    fn try_from(value: Table) -> Result<Self, Self::Error> {
        let type_ = value.get::<String>(1)?;
        match type_.as_str() {
            "shapeColor" => {
                let shape: Shape = value.get::<Table>("shape")?.try_into()?;
                let color: Color = value.get::<Table>("color")?.try_into()?;
                Ok(RenderCommand::ShapeColor(shape, color))
            }
            _ => Err(Self::Error::RuntimeError(format!("Unknown shape: {}", type_))),
        }
    }
}

impl UserData for UIRenderContext {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("push", |_, this, (table): (Table)| {
            let command: RenderCommand = table.try_into()?;
            this.list.push(command);
            Ok(())
        });
        methods.add_method("size", |lua, this, ()| {
            let table = lua.create_table()?;
            let (w, h) = this.nvg.borrow().relative(1.0, 1.0);
            table.set("w", w)?;
            table.set("h", h)?;
            Ok(table)
        });
        methods.add_method_mut("newText", |_, this, (text, font, size): (String, String, f32)| {
            let text = this.nvg.borrow_mut().text(text.as_str(), &font, size);
            Ok(text)
        });
        methods.add_method_mut("newVideoSurface", |_, this, (): ()| {
            Ok(VideoSurface::new())
        });
    }
}

impl IntoLua for InputEvent {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        match self {
            InputEvent::Key(key, action) => {
                table.set(1, "key")?;
                table.set(2, key as i32)?;
                table.set(3, action as i32)?;
            }
            InputEvent::MouseMoved((x0, y0), (x, y)) => {
                table.set(1, "mouseMove")?;
                table.set(2, x0 as i32)?;
                table.set(3, y0 as i32)?;
                table.set(4, x as i32)?;
                table.set(5, y as i32)?;
            }
            InputEvent::MouseButton(button, action, (x0, y0)) => {
                table.set(1, "mouseButton")?;
                table.set(2, button as i32)?;
                table.set(3, action as i32)?;
                table.set(4, x0 as i32)?;
                table.set(5, y0 as i32)?;
            }
        }
        Ok(Value::Table(table))
    }
}

pub struct UIManager {
    lua: Lua,
    window_size: (f32, f32),
    mouse_position: (f32, f32),
    render_function: Option<Function>,
    update_function: Option<Function>,
    event_function: Option<Function>,
}

impl UIManager {
    pub fn new(nvg: Rc<RefCell<NvgContext>>, window: &Window) -> Self {
        let lua = Lua::new();
        let globals = lua.globals();
        globals.set("ui", UIRenderContext::new(nvg)).unwrap();
        globals.set("dirty", true).unwrap();
        let (w, h) = window.get_framebuffer_size();
        Self {
            lua,
            render_function: None,
            update_function: None,
            event_function: None,
            window_size: (w as f32, h as f32),
            mouse_position: (0.0, 0.0),
        }
    }

    pub fn load_script(&mut self, chunk: impl AsChunk) -> Result<(), mlua::Error> {
        let table: Table = self.lua.load(chunk).eval()?;
        self.render_function = Some(table.get("render")?);
        self.update_function = Some(table.get("update")?);
        self.event_function = Some(table.get("event")?);
        Ok(())
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn render(&self, width: f32, height: f32) -> Result<(), mlua::Error> {
        let globals = self.lua.globals();
        let ui = globals.get::<AnyUserData>("ui")?.borrow::<UIRenderContext>()?;

        let mut nvg = ui.nvg.borrow_mut();
        nvg.set_size((width, height));
        drop(nvg);
        drop(ui);
        if globals.get::<bool>("dirty")? {
            globals.set("dirty", false)?;
            if let Some(render) = &self.render_function {
                render.call::<()>(())?;
            }
        }
        if let Some(update) = &self.update_function {
            update.call::<()>(())?;
        }

        let ui = globals.get::<AnyUserData>("ui")?.borrow::<UIRenderContext>()?;
        let mut nvg = ui.nvg.borrow_mut();
        nvg.begin_frame((width, height));

        for cmd in ui.list.iter() {
            match *cmd {
                RenderCommand::ShapeColor(shape, color) => {
                    nvg.begin_path();
                    nvg.draw_shape(shape);
                    nvg.fill_color(color);
                    nvg.fill();
                }
            }
        }

        nvg.end_frame();
        Ok(())
    }

    pub fn handle_event(&mut self, event: WindowEvent) -> mlua::Result<()> {
        let event = match event.clone() {
            WindowEvent::FramebufferSize(w, h) => {
                self.set_dirty()?;
                self.window_size = (w as f32, h as f32);
                None
            }
            WindowEvent::CursorPos(x, y) => {
                let to = (x as f32, y as f32);
                let from = self.mouse_position;
                self.mouse_position = to;
                Some(InputEvent::MouseMoved(from, to))
            }
            WindowEvent::Key(key, _, action, _) => {
                Some(InputEvent::Key(key, action))
            }
            _ => None
        };
        if let Some((on_event, event)) = self.event_function.as_ref().zip(event) {
            on_event.call::<()>(event)?;
        }
        Ok(())
    }

    pub fn set_dirty(&self) -> Result<(), mlua::Error> {
        self.lua.globals().set("dirty", true)?;
        Ok(())
    }
}