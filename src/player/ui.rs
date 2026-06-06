use crate::gs::nvg;
use crate::gs::nvg::{Color, Image, NvgContext, Point, Shape, Text, TextHorizontalAlignment, TextVerticalAlignment};
use crate::gs::window::Window;
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton, WindowEvent};
use mlua::prelude::LuaTable;
use mlua::{AnyUserData, AsChunk, Function, Lua, Table, UserData, UserDataMethods, Value};
use std::cell::RefCell;
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

pub enum RenderCommand {
    VideoSurfaceDraw(Shape, AnyUserData, Option<Image>),
    ShapeColor(Shape, Color),
    TextBox(Text, Color, Shape, TextHorizontalAlignment, TextVerticalAlignment),
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

pub struct UIManager {
    commands: Vec<RenderCommand>,
    lua: Lua,
    root: Option<LuaTable>,
    update_functions: Vec<(Table, Function)>,
    bodies: Vec<(Shape, Table)>,
}

impl UIManager {
    pub fn new() -> Self {
        let lua = Lua::new();
        let globals = lua.globals();
        globals.set("inputWorker", InputWorker::new()).unwrap();
        globals.set("decodeWorker", DecodeWorker::new()).unwrap();
        globals.set("createPlayer", lua.create_function(|lua, (path, surface): (Value, Value)| {
            let path = path.as_string().unwrap().to_string_lossy();
            let input_worker: AnyUserData = lua.globals().get("inputWorker").unwrap();
            let decode_worker: AnyUserData = lua.globals().get("decodeWorker").unwrap();
            let input = Input::open(&path, &[]).unwrap();
            let mut surface = surface.as_userdata().unwrap().borrow_mut::<VideoSurface>().unwrap();
            let player = VideoPlayer::new(
                input,
                Some(&mut *surface),
                &mut *decode_worker.borrow_mut().unwrap(),
                &mut *input_worker.borrow_mut().unwrap()
            ).unwrap();
            Ok(player)
        }).unwrap()).unwrap();
        Self {
            commands: Vec::new(),
            lua,
            root: None,
            update_functions: Vec::new(),
            bodies: Vec::new(),
        }
    }

    pub fn exec(&mut self, chunk: impl AsChunk) {
        self.lua.load(chunk).exec().unwrap();
        self.dirty();
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn run_updates(&mut self) -> Result<(), mlua::Error> {
        for (parent, function) in self.update_functions.iter() {
            function.call::<()>(parent)?;
        }
        Ok(())
    }

    pub fn update(&mut self, nvg: &mut NvgContext) -> Result<(), mlua::Error> {
        let globals = self.lua.globals();
        if !globals.get::<Value>("dirty")?.as_boolean().unwrap_or(false) {
            return Ok(());
        }
        globals.set("dirty", false)?;
        self.commands.clear();
        self.update_functions.clear();
        self.bodies.clear();
        self.root = globals.get("root").ok();

        if let Some(root) = self.root.clone() {
            let size = self.vec_table(nvg.width(None), nvg.height(None)).unwrap();
            let pos = self.vec_table(0.0, 0.0).unwrap();
            root.set("size", size.clone())?;
            root.set("pos", pos.clone())?;
            self.update_component(root, nvg).unwrap();
        }
        Ok(())
    }

    fn update_component(&mut self, component: Table, nvg: &mut NvgContext) -> Option<()> {
        let typ: Value = component.get("type").ok()?;
        let update: Value = component.get("update").ok()?;
        if !update.is_nil() {
            if let Some(update) = update.as_function() {
                self.update_functions.push((component.clone(), update.clone()));
            }
        }
        let on_dirty: Value = component.get("onDirty").ok()?;
        if !on_dirty.is_nil() {
            if let Some(on_dirty) = on_dirty.as_function() {
                on_dirty.call::<()>(component.clone()).unwrap();
            }
        }

        match typ.as_string().unwrap().to_string_lossy().as_str() {
            "root" => {
                let (x, y, w, h) = Self::component_bounds(&component)?;

                if let Some(children) = component.get::<Table>("children").ok() {
                    for child in children.sequence_values::<Table>().map(|v| v.unwrap()) {
                        let size = self.vec_table(w, h)?;
                        let pos = self.vec_table(x, y)?;
                        child.set("size", size).ok()?;
                        child.set("pos", pos).ok()?;

                        child.set("parent", component.clone()).ok()?;
                        self.update_component(child, nvg);
                    }
                }
            }
            "group" => {
                let group_flow = Self::component_get_string(&component, "flow")?;
                let children = component.get::<Table>("children").ok()?;
                let mut sum = 0.0f32;
                for child in children.sequence_values::<Table>() {
                    let child = child.ok()?;
                    let w: Value = child.get(1).ok()?;
                    sum += w.as_f32()?;
                }
                let (mut x, mut y, w, h) = Self::component_bounds(&component)?;
                for child in children.sequence_values::<Table>() {
                    let (weight, child) = child.ok().and_then(|c| {
                        Some((c.get::<Value>(1).ok()?.as_f32()? / sum, c.get::<Table>(2).ok()?))
                    })?;
                    child.set("parent", component.clone()).ok()?;
                    let (dx, w) = match group_flow.as_str() {
                        "h" | "hor" | "ho" => (weight * w, weight * w),
                        _ => (0.0, w)
                    };
                    let (dy, h) = match group_flow.as_str() {
                        "v" | "ver" | "ve" | "vert" => (weight * h, weight * h),
                        _ => (0.0, h)
                    };
                    child.set("pos", self.vec_table(x, y)).ok()?;
                    child.set("size", self.vec_table(w, h)).ok()?;
                    self.update_component(child, nvg);
                    x += dx;
                    y += dy;
                }
            }
            "rect" => {
                let color = Self::from_table_color(component.get("color").ok()?)?;
                let (x, y, w, h) = Self::component_bounds(&component)?;
                self.commands.push(RenderCommand::ShapeColor(Shape::Rect(x, y, w, h), color));
            }
            "videoSurface" => {
                let surface: Value = component.get::<Value>("surface").ok()?;
                if surface.is_nil() {
                    component.set("surface", VideoSurface::new()).ok()?
                }
                let surface = component.get::<AnyUserData>("surface").ok()?;
                let (x, y, w, h) = Self::component_bounds(&component)?;
                self.commands.push(RenderCommand::VideoSurfaceDraw(Shape::Rect(x, y, w, h), surface, None))
            }
            "raw" => {
                let render = component.get::<Function>("render").ok()?;
                let result = render.call::<Value>(component).ok()?;
                for child in result.as_table()?.sequence_values::<Table>() {
                    self.commands.push(Self::get_raw_render_command(&child.ok()?)?);
                }
            }
            _ => {}
        }
        Some(())
    }

    fn get_raw_render_command(table: &Table) -> Option<RenderCommand> {
        let name = table.get::<String>(1).ok()?;
        match name.as_str() {
            "shapeColor" => {
                let color = Self::from_table_color(table.get("color").ok()?)?;
                let shape = Self::from_table_shape(table.get("shape").ok()?)?;
                Some(RenderCommand::ShapeColor(shape, color))
            }
            _ => None
        }
    }

    fn from_table_shape(table: Table) -> Option<Shape> {
        let name = table.get::<String>("name").ok()?;
        match name.as_str() {
            "rect" => {
                let x: f32 = table.get(1).ok()?;
                let y: f32 = table.get(2).ok()?;
                let w: f32 = table.get(3).ok()?;
                let h: f32 = table.get(4).ok()?;
                Some(Shape::Rect(x, y, w, h))
            }
            _ => None
        }
    }

    pub fn dirty(&mut self) {
        self.lua.globals().set("dirty", true).unwrap();
    }

    fn from_table_color(table: Table) -> Option<Color> {
        let r: Value = table.get(1).ok()?;
        let g: Value = table.get(2).ok()?;
        let b: Value = table.get(3).ok()?;
        let a: Value = table.get(4).ok()?;
        Some(Color::rgba(r.as_f32()?, g.as_f32()?, b.as_f32()?, a.as_f32()?))
    }

    fn from_table_vec(table: Table) -> Option<(f32, f32)> {
        let x: Value = table.get(1).ok()?;
        let y: Value = table.get(2).ok()?;
        Some((x.as_f32()?, y.as_f32()?))
    }

    fn component_bounds(component: &Table) -> Option<(f32, f32, f32, f32)> {
        let (px, py) = Self::component_get_vec_opt(component, "prefPos").unwrap_or((None, None));
        let (pw, ph) = Self::component_get_vec_opt(component, "prefSize").unwrap_or((None, None));
        let (w, h) = Self::component_get_vec(component, "size")?;
        let (x, y) = Self::component_get_vec(component, "pos")?;

        Some((px.unwrap_or(x), py.unwrap_or(y), pw.unwrap_or(w), ph.unwrap_or(h)))
    }

    fn vec_table(&self, x: f32, y: f32) -> Option<Table> {
        let table = self.lua.create_table().ok()?;
        table.set(1, x).ok()?;
        table.set(2, y).ok()?;
        Some(table)
    }

    fn component_get_pref_or_vec(component: &Table, pref: &str, or: &str) -> Option<(f32, f32)> {
        Self::component_get_vec(component, pref)
            .or_else(|| Self::component_get_vec(component, or))
    }

    fn component_get_vec_opt(component: &Table, name: &str) -> Option<(Option<f32>, Option<f32>)> {
        let table: Table = component.get(name).ok()?;
        let x = table.get::<f32>(1).ok();
        let y = table.get::<f32>(2).ok();
        Some((x, y))
    }

    fn component_get_vec(component: &Table, name: &str) -> Option<(f32, f32)> {
        let table = component.get(name).ok()?;
        Self::from_table_vec(table)
    }

    fn component_get_string(component: &Table, name: &str) -> Option<String> {
        let value: Value = component.get(name).ok()?;
        value.as_string().map(|s| s.to_string_lossy())
    }

    fn get_body(component: Table) -> Option<(Shape, Table)> {
        todo!()
    }

    pub fn render(&mut self, nvg: &mut NvgContext) {
        for command in self.commands.iter_mut() {
            match command {
                RenderCommand::VideoSurfaceDraw(shape, comp, image) => {
                    let mut surface = comp.borrow_mut::<VideoSurface>().unwrap();
                    surface.update();
                    if let Some(_) = surface.size_update.take() {
                        if let Some(image) = image.take() {
                            nvg.delete_image(image);
                        }
                        *image = nvg.create_texture_image(&surface.output_texture);
                    }
                    if image.is_none() {
                        *image = nvg.create_texture_image(&surface.output_texture);
                    }
                    if let Some(image) = image {
                        nvg.begin_path();
                        let (_, _, w, h) = shape.bounds();
                        let (pw, ph) = image.size_conserve_aspect_ratio(w, h);
                        let ox = (w - pw) / 2.0;
                        let oy = (h - ph) / 2.0;
                        let shape = shape.scale_xy(pw / w, ph / h, true);
                        let paint = nvg.image_paint(image, shape, 1.0);
                        nvg.draw_shape(shape);
                        nvg.fill_paint(paint);
                        nvg.fill();
                    }
                }
                RenderCommand::ShapeColor(shape, color) => {
                    nvg.begin_path();
                    nvg.draw_shape(*shape);
                    nvg.fill_color(*color);
                    nvg.fill();
                }
                RenderCommand::TextBox(text, color, shape, width, height) => {
                    nvg.begin_path();
                    nvg.fill_color(*color);
                    let (_, _, w, h) = shape.bounds();
                    nvg.fit_text(text, w, h);
                    nvg.draw_text_inside(text, *shape, *width, *height);
                    nvg.fill();
                }
            }
        }
    }

}