use crate::gs::nvg::{Color, Image, NvgContext, Shape, Text};
use crate::gs::window::Window;
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton, WindowEvent};
use mlua::{AnyUserData, AsChunk, Function, IntoLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::RefCell;
use std::rc::Rc;
use crate::ffmpeg::input::Input;
use crate::player::player::VideoPlayer;

pub enum InputEvent {
    MouseMoved((f32, f32), (f32, f32)),
    MouseButton(MouseButton, Action, (f32, f32)),
    Key(Key, Action)
}

#[derive(Clone)]
pub enum RenderCommand {
    ShapeColor(Shape, Color),
    VideoSurface(AnyUserData, Shape, Option<Image>),
    Indirect(Table, Option<Box<RenderCommand>>)
}

impl UserData for RenderCommand {}

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
    render_commands: Vec<RenderCommand>,
    nvg: Rc<RefCell<NvgContext>>,
    frame_buffer_size: (f32, f32)
}

impl UIRenderContext {
    fn new(nvg: Rc<RefCell<NvgContext>>) -> Self {
        let size = nvg.borrow().relative(1.0, 1.0);
        Self { render_commands: Vec::new(), nvg, frame_buffer_size: size }
    }

    pub fn render(&mut self) -> Result<(), mlua::Error> {
        let mut nvg = self.nvg.borrow_mut();
        nvg.begin_frame(self.frame_buffer_size);
        for cmd in self.render_commands.iter_mut() {
            Self::render_command(cmd, &mut *nvg)?
        }
        nvg.end_frame();
        Ok(())
    }

    pub fn render_command(command: &mut RenderCommand, nvg: &mut NvgContext) -> Result<(), mlua::Error> {
        match command {
            RenderCommand::ShapeColor(shape, color) => {
                nvg.begin_path();
                nvg.draw_shape(*shape);
                nvg.fill_color(*color);
                nvg.fill();
            }
            RenderCommand::VideoSurface(surface, shape, image) => {
                let mut surface = surface.borrow_mut::<VideoSurface>()?;
                surface.update();
                if let Some((_, image)) = surface.size_update.take().zip(image.take()) {
                    nvg.delete_image(image)
                }
                if image.is_none() {
                    *image = nvg.create_texture_image(&surface.output_texture)
                }
                if let Some(image) = image {
                    nvg.begin_path();
                    let (x, y, w, h) = shape.bounds();
                    let (pw, ph) = image.size_conserve_aspect_ratio(w, h);
                    let (ox, oy) = ((w - pw) / 2.0, (h - ph) / 2.0);
                    let shape = shape.scale_xy(pw / w, ph / h, false).translate(ox, oy);
                    nvg.begin_path();
                    nvg.draw_shape(shape);
                    let paint = nvg.image_paint(image, shape, 1.0);
                    nvg.fill_paint(paint);
                    nvg.fill();
                }
            }
            RenderCommand::Indirect(table, command) => {
                if table.get::<bool>("dirty")? {
                    table.set("dirty", false)?;
                    command.take();
                }
                if command.is_none() {
                    let cmd = table.clone().try_into()?;
                    *command = Some(Box::new(cmd));
                }
                if let Some(command) = command {
                    Self::render_command(&mut *command, nvg)?;
                }
            }
        }
        Ok(())
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
            "videoSurface" => {
                let surface: AnyUserData = value.get::<AnyUserData>("surface")?;
                let shape: Shape = value.get::<Table>("shape")?.try_into()?;
                Ok(RenderCommand::VideoSurface(surface, shape, None))
            }
            "indirect" => {
                let command = value.get::<Table>("command")?;
                Ok(RenderCommand::Indirect(command, None))
            }
            _ => Err(Self::Error::RuntimeError(format!("Unknown shape: {}", type_))),
        }
    }
}

impl UserData for UIRenderContext {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("push", |_, this, (table): (Table)| {
            let command: RenderCommand = table.try_into()?;
            this.render_commands.push(command);
            Ok(())
        });
        methods.add_method("size", |lua, this, ()| {
            let table = lua.create_table()?;
            let (w, h) = this.frame_buffer_size;
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
        methods.add_method("newVideoPlayer", |lua, this, (path, surface): (String, AnyUserData)| {
            let mut surface = surface.borrow_mut::<VideoSurface>()?;
            let table = lua.create_table()?;
            let mut input_worker = InputWorker::new();
            let mut decode_worker = DecodeWorker::new();
            let input = Input::open(&path, &[]).unwrap();
            let player = VideoPlayer::new(input, Some(&mut *surface), &mut decode_worker, &mut input_worker).unwrap();
            table.set("input", input_worker)?;
            table.set("decode", decode_worker)?;
            table.set("player", player)?;
            return Ok(table);
        })
    }
}

impl IntoLua for InputEvent {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        match self {
            InputEvent::Key(key, action) => {
                table.set("type", "key")?;
                table.set("key", key as i32)?;
                table.set("action", action as i32)?;
            }
            InputEvent::MouseMoved((x0, y0), (x, y)) => {
                table.set("type", "mouseMoved")?;
                let from = lua.create_table()?;
                from.set("x", x0)?;
                from.set("y", y0)?;
                let to = lua.create_table()?;
                to.set("x", x)?;
                to.set("y", y)?;
                table.set("from", from)?;
                table.set("to", to)?;
            }
            InputEvent::MouseButton(button, action, (x0, y0)) => {
                let pos = lua.create_table()?;
                pos.set("x", x0)?;
                pos.set("y", y0)?;
                table.set("type", "mouseButton")?;
                table.set("button", button as i32)?;
                table.set("action", action as i32)?;
                table.set("pos", pos)?;
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
        let size = self.lua.create_table()?;
        size.set("w", width)?;
        size.set("h", height)?;
        globals.set("size", size)?;
        if globals.get::<bool>("dirty")? {
            globals.set("dirty", false)?;
            if let Some(render) = &self.render_function {
                let mut ui = globals.get::<AnyUserData>("ui")?.borrow_mut::<UIRenderContext>()?;
                ui.render_commands.clear();
                drop(ui); // Stop the damn thing from whining. I mean why the hell can't you have more than one mut borrows here?
                render.call::<()>(())?;
            }
        }
        if let Some(update) = &self.update_function {
            update.call::<()>(())?;
        }

        let mut ui = globals.get::<AnyUserData>("ui")?.borrow_mut::<UIRenderContext>()?;
        ui.frame_buffer_size = (width, height);
        ui.render()?;
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
                let to = (x as f32, self.window_size.1 - y as f32);
                let from = self.mouse_position;
                self.mouse_position = to;
                Some(InputEvent::MouseMoved(from, to))
            }
            WindowEvent::Key(key, _, action, _) => {
                Some(InputEvent::Key(key, action))
            }
            WindowEvent::MouseButton(button, action, _) => {
                Some(InputEvent::MouseButton(button, action, self.mouse_position))
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