use crate::gs::nvg::{Color, Image, NvgInstance, Shape, Text, TextFitType, TextOld};
use crate::gs::window::Window;
use crate::player::decoder::DecodeWorker;
use crate::player::input::InputWorker;
use crate::player::surface::VideoSurface;
use glfw::{Action, Key, MouseButton, WindowEvent};
use mlua::{AnyUserData, AsChunk, Function, IntoLua, IntoLuaMulti, LightUserData, Lua, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value};
use std::cell::{Cell, RefCell, RefMut};
use std::ops::Deref;
use std::rc::Rc;
use std::time::{Duration, Instant};
use ffmpeg_sys_next::MQ_PRIO_MAX;
use mlua::prelude::LuaValue;
use crate::ffmpeg::input::Input;
use crate::player::player::VideoPlayer;

pub enum InputEvent {
    MouseMoved((f32, f32), (f32, f32)),
    MouseButton(MouseButton, Action, (f32, f32)),
    Key(Key, Action),
    Char(char),
}

#[derive(Clone)]
pub enum RenderCommand {
    ShapeFillColor(Shape, Color),
    ShapeStrokeColor(Shape, f32, Color),
    VideoSurface(AnyUserData, Shape, Option<Image>),
    TextFill(AnyUserData, Color),
    Indirect(Rc<RefCell<RenderCommand>>),
}

impl UserData for RenderCommand {}

impl UserData for VideoSurface {}
impl UserData for InputWorker {}
impl UserData for DecodeWorker {}
impl UserData for VideoPlayer {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("pts", |_, this| {
            Ok(this.current_pts())
        });
        fields.add_field_method_get("duration", |_, this| {
            Ok(this.estimated_duration)
        });
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("play", |_, this, _args: ()| {
            this.play();
            Ok(())
        });
        methods.add_method_mut("seek", |_, this, (target): (f64)| {
            this.seek(target);
            Ok(())
        });
    }

}

impl Into<&str> for TextFitType {
    fn into(self) -> &'static str {
        match self {
            TextFitType::Scale => "scale",
            TextFitType::Range => "range",
            TextFitType::RangeAndScale => "rangeAndScale",
        }
    }
}

impl IntoLua for TextFitType {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let str: &str = self.into();
        Ok(Value::String(lua.create_string(str)?))
    }
}

impl UserData for Text {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("offset", |lua, this| {
            Ok(this.get_offset())
        });
        fields.add_field_method_set("offset", |_, this, offset: Value| {
            this.set_offset(offset.as_usize().unwrap_or(0));
            Ok(())
        });
        fields.add_field_method_get("pos", |lua, this| {
            let (x, y) = this.get_pos();
            Ok(lua.create_table_from([
                ("x", x),
                ("y", y),
            ])?)
        });
        fields.add_field_method_set("pos", |_, this, table: Table| {
            this.set_pos((table.get("x")?, table.get("y")?));
            Ok(())
        });
        fields.add_field_method_set("size", |_, this, value: Value| {
            let size = if let Some(table) = value.as_table() {
                Some((table.get("w")?, table.get("h")?))
            } else {
                None
            };
            this.set_fit_size(size);
            Ok(())
        });
        fields.add_field_method_get("size", |lua, this| {
            let size = this.get_fit_size();
            let out = if let Some((w, h)) = size {
                Value::Table(lua.create_table_from([
                    ("w", w),
                    ("h", h),
                ])?)
            } else {
                Value::Nil
            };
            Ok(out)
        });
        fields.add_field_method_get("len", |_, this| {
            Ok(this.len())
        });
        fields.add_field_method_get("text", |lua, this| {
            Ok(lua.create_string(this.get_text())?)
        });
        fields.add_field_method_set("text", |_, this, string: mlua::String| {
            this.set_text(string.to_str()?.as_ref());
            Ok(())
        });
        fields.add_field_method_set("font", |_, this, table: Table| {
            let name: mlua::String = table.get("name")?;
            let font_size: f32 = table.get("size")?;
            this.set_font((name.to_str()?.as_ref(), font_size));
            Ok(())
        });
        fields.add_field_method_get("fitType", |lua, this| {
            let str = match this.get_fit_type() {
                TextFitType::Scale => "scale",
                TextFitType::Range => "range",
                TextFitType::RangeAndScale => "rangeAndScale",
            };
            Ok(lua.create_string(str)?)
        });
        fields.add_field_method_set("fitType", |lua, this, ty: mlua::String| {
            let str = ty.to_str()?;
            let ty = match str.as_ref() {
                "scale" => TextFitType::Scale,
                "range" => TextFitType::Range,
                "rangeAndScale" => TextFitType::RangeAndScale,
                _ => return Err(mlua::Error::RuntimeError(format!("Invalid text fit type: {}", str))),
            };
            this.set_fit_type(ty);
            Ok(())
        })
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("clear", |_, this, ()| {
            this.with_string(|dest| {
                dest.clear();
            });
            this.set_offset(0);
            Ok(())
        });
        methods.add_method("bounds", |lua, this, (begin, end): (Value, Value)| {
            let begin = begin.as_usize().unwrap_or(0);
            let end = end.as_usize().unwrap_or(this.len());
            let (x, y, w, h) = this.range_bounds(Some(begin..end));
            let (px, ..) = this.get_pos();
            let x1 = this.display_range_offset().map(|ox| px + x - ox).unwrap_or(x);
            let table = lua.create_table_from([
                (1, x1),
                (2, y),
                (3, w),
                (4, h),
            ])?;
            Ok(table)
        });
        methods.add_method_mut("push", |_, this, string: mlua::String| {
            let str = string.to_str()?;
            let offset = this.get_offset();
            this.with_string(|dest| {
                dest.insert_str(offset, &str);
            });
            this.set_offset(offset + str.len());
            Ok(())
        });
        methods.add_method_mut("pop", |_, this, ()| {
            let offset = this.get_offset().max(1) - 1;
            this.with_string(|dest| {
                if offset < dest.len() {
                    dest.remove(offset);
                }
            });
            this.set_offset(offset);
            Ok(())
        });
    }
}

impl UserData for TextOld {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_set("x", |_, this, x: f32| {
            this.x = x;
            Ok(())
        });
        fields.add_field_method_set("y", |_, this, y: f32| {
            this.y = y;
            Ok(())
        });
        fields.add_field_method_get("len", |_, this| {
            Ok(this.len())
        });
        fields.add_field_method_set("range", |lua, this, range: Value| {
            this.range = if let Some(range) = range.as_table() {
                let begin = range.get::<Value>(1)?.as_usize();
                let end = range.get::<Value>(2)?.as_usize();
                if begin.is_none() && end.is_none() {
                    None
                } else {
                    let begin = begin.unwrap_or(0).max(0).min(this.len());
                    let end = end.unwrap_or(this.len()).max(0).min(this.len());
                    Some(begin.min(end)..end.max(begin))
                }
            } else {
                None
            };
            Ok(())
        });
        fields.add_field_method_get("range", |lua, this| {
            if let Some(range) = this.range.as_ref() {
                let table = lua.create_table_from([
                    (1, range.start),
                    (2, range.end),
                ])?;
                return Ok(Value::Table(table));
            }
            Ok(Value::Nil)
        })
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("bounds", |lua, this, (begin, end): (Option<usize>, Option<usize>)| {
            let result: (f32, f32, f32, f32) = if begin.is_none() && end.is_none() {
                this.bounds()
            } else {
                let begin = begin.unwrap_or(0);
                let end = end.unwrap_or(this.len());
                this.char_range_bounds(begin..end)
            };

            Ok(Value::Table(lua.create_table_from([
                (1, result.0),
                (2, result.1),
                (3, result.2),
                (4, result.3),
            ])?))
        });
    }
}

struct Task {
    last: Option<Instant>,
    canceled: Rc<RefCell<bool>>,
    predicate: Function,
    function: Function,
}

struct TaskHandle {
    canceled: Rc<RefCell<bool>>,
}

impl UserData for TaskHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("canceled", |_, this| Ok(this.canceled.borrow().clone()));
        fields.add_field_method_set("canceled", |_, this, canceled: bool| {
            this.canceled.replace(canceled);
            Ok(())
        });
    }
}

impl Task {
    fn new(predicate: Function, function: Function) -> Self {
        let last = Instant::now();
        Self {
            last: None,
            canceled: Rc::new(RefCell::new(false)),
            predicate,
            function,
        }
    }
    
    fn is_canceled(&self) -> bool {
        *self.canceled.borrow()
    }
    
    fn check(&self) -> Result<bool, mlua::Error> {
        if self.last.is_none() {
            return Ok(true)
        }
        self.predicate.call(self.last.unwrap().elapsed().as_secs_f64())
    }
    
    fn run(&mut self) -> Result<(), mlua::Error> {
        self.function.call::<()>(())?;
        self.last.replace(Instant::now());
        Ok(())
    }
    
    fn handle(&self) -> TaskHandle {
        TaskHandle {
            canceled: self.canceled.clone(),
        }
    }
}

struct UIRenderContext {
    render_commands: Vec<RenderCommand>,
    nvg: NvgInstance,
    frame_buffer_size: (f32, f32),
    
    tasks: Vec<Task>,

    render_handle: Option<Function>,
    event_handle: Option<Function>,
    update_handle: Option<Function>,
    dirty: bool
}

impl UIRenderContext {
    fn new(nvg: &NvgInstance) -> Self {
        let nvg = nvg.clone();
        let size = nvg.relative(1.0, 1.0);
        Self {
            render_commands: Vec::new(),
            nvg,
            frame_buffer_size: size,
            tasks: Vec::new(),
            render_handle: None,
            event_handle: None,
            update_handle: None,
            dirty: true
        }
    }

    pub fn render(&mut self) -> Result<(), mlua::Error> {
        if let Some((dirty, render_handle)) = Some(&mut self.dirty)
            .take_if(|d| **d).zip(self.render_handle.as_ref()) {
            *dirty = false;
            self.render_commands.clear();
            render_handle.call::<()>(())?;
        }
        if let Some(update_handle) = self.update_handle.as_ref() {
            update_handle.call::<()>(())?;
        }

        let nvg = &mut self.nvg;
        nvg.begin_frame(self.frame_buffer_size);
        for cmd in self.render_commands.iter_mut() {
            Self::render_command(cmd, nvg)?
        }
        nvg.end_frame();
        
        self.tasks.retain(|t| !t.is_canceled());
        for task in self.tasks.iter_mut() {
            if task.check()? { task.run()?; }
        }
        
        Ok(())
    }

    pub fn render_command(command: &mut RenderCommand, nvg: &mut NvgInstance) -> Result<(), mlua::Error> {
        match command {
            RenderCommand::ShapeFillColor(shape, color) => {
                nvg.begin_path();
                nvg.draw_shape(*shape);
                nvg.fill_color(*color);
                nvg.fill();
            }
            RenderCommand::ShapeStrokeColor(shape, width, color) => {
                nvg.begin_path();
                nvg.draw_shape(*shape);
                nvg.stroke_color(*color);
                nvg.stroke_width(*width);
                nvg.stroke();
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
            RenderCommand::Indirect(command) => {
                let command = &mut *command.borrow_mut();
                Self::render_command(command, nvg)?;
            },
            RenderCommand::TextFill(text, color) => {
                nvg.begin_path();
                nvg.fill_color(*color);
                let text = text.borrow_mut::<Text>()?;
                text.draw(nvg);
                nvg.fill();
            },
        }
        Ok(())
    }

    pub fn handle_event(&mut self, event: InputEvent) -> Result<(), mlua::Error> {
        if let Some(event_handler) = self.event_handle.as_ref() {
            event_handler.call::<()>(event)?;
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
        let type_ = value.get::<mlua::String>(1)?;
        match type_.to_str()?.as_ref() {
            "rect" => {
                let x = value.get::<f32>(2)?;
                let y = value.get::<f32>(3)?;
                let width = value.get::<f32>(4)?;
                let height = value.get::<f32>(5)?;
                Ok(Shape::Rect(x, y, width, height))
            }
            _ => Err(Self::Error::RuntimeError(format!("Unknown shape: {}", type_.to_str()?.as_ref()))),
        }
    }
}

impl TryFrom<Table> for RenderCommand {
    type Error = mlua::Error;

    fn try_from(value: Table) -> Result<Self, Self::Error> {
        let type_ = value.get::<mlua::String>("type")?;
        match type_.to_str()?.as_ref() {
            "shapeFillColor" => {
                let shape: Shape = value.get::<Table>("shape")?.try_into()?;
                let color: Color = value.get::<Table>("color")?.try_into()?;
                Ok(RenderCommand::ShapeFillColor(shape, color))
            }
            "shapeStrokeColor" => {
                let shape: Shape = value.get::<Table>("shape")?.try_into()?;
                let width = value.get("width")?;
                let color: Color = value.get::<Table>("color")?.try_into()?;
                Ok(RenderCommand::ShapeStrokeColor(shape, width, color))
            }
            "videoSurface" => {
                let surface: AnyUserData = value.get::<AnyUserData>("surface")?;
                let shape: Shape = value.get::<Table>("shape")?.try_into()?;
                Ok(RenderCommand::VideoSurface(surface, shape, None))
            }
            "textFill" => {
                let text: AnyUserData = value.get::<AnyUserData>("text")?;
                let color = value.get::<Table>("color")?.try_into()?;
                Ok(RenderCommand::TextFill(text, color))
            }
            _ => Err(Self::Error::RuntimeError(format!("Unknown shape: {}", type_.to_str()?.as_ref()))),
        }
    }
}

struct IndirectCommandHandle {
    command: Rc<RefCell<RenderCommand>>,
    command_table: Table
}

impl IndirectCommandHandle {
    fn new(command: RenderCommand, current: Table) -> Self {
        Self { command: Rc::new(RefCell::new(command)), command_table: current }
    }

    pub fn clone_ref(&self) -> Rc<RefCell<RenderCommand>> {
        self.command.clone()
    }
}

impl UserData for IndirectCommandHandle {

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("update", |_, this, ()| {
            let current = this.command_table.clone();
            let command: RenderCommand = current.try_into()?;
            this.command.replace(command);
            Ok(())
        });
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("command", |_, this| {
            Ok(this.command_table.clone())
        });
        fields.add_field_method_set("command", |_, this, command: Table| {
            this.command_table = command.clone();
            let command: RenderCommand = command.try_into()?;
            this.command.replace(command);
            Ok(())
        });
    }

}

#[derive(Copy, Clone)]
struct UIRenderContextRef(*mut UIRenderContext);

impl UIRenderContextRef {
    fn new(nvg: &NvgInstance) -> Self {
        let rc = Box::new(UIRenderContext::new(nvg));
        UIRenderContextRef(Box::into_raw(rc))
    }

    fn get(&self) -> &mut UIRenderContext {
        unsafe { &mut *(self.0) }
    }

    fn userdata(&self) -> LightUserData {
        LightUserData(self.0 as *mut _)
    }

    fn deallocate(&self) {
        unsafe {
            drop(Box::from_raw(self.0));
        }
    }
}

impl From<LightUserData> for UIRenderContextRef {
    fn from(userdata: LightUserData) -> Self {
        UIRenderContextRef(userdata.0 as *mut _)
    }
}

impl UserData for UIRenderContextRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("push", |_, this, (table): (Table)| {
            let command: RenderCommand = table.try_into()?;
            this.get().render_commands.push(command);
            Ok(())
        });
        methods.add_method_mut("pushIndirect", |_, this, (table): (Table)| {
            let handle = IndirectCommandHandle::new(table.clone().try_into()?, table);
            this.get().render_commands.push(RenderCommand::Indirect(handle.clone_ref()));
            Ok(handle)
        });
        methods.add_method_mut("setDirty", |_, this, (): ()| {
            this.get().dirty = true;
            Ok(())
        });
        methods.add_method_mut("setRenderHandle", |_, this, (handle): (Function)| {
            this.get().render_handle = Some(handle);
            Ok(())
        });
        methods.add_method_mut("setEventHandle", |_, this, (handle): (Function)| {
            this.get().event_handle = Some(handle);
            Ok(())
        });
        methods.add_method_mut("setUpdateHandle", |_, this, (handle): (Function)| {
            this.get().update_handle = Some(handle);
            Ok(())
        });
        methods.add_method("getSize", |lua, this, ()| {
            let table = lua.create_table()?;
            let (w, h) = this.get().frame_buffer_size;
            table.set("w", w)?;
            table.set("h", h)?;
            Ok(table)
        });
        methods.add_method_mut("newText", |_, this, (init_text, font, size): (mlua::String, mlua::String, f32)| {
            let mut text = Text::new(&this.get().nvg, (font.to_str()?.as_ref(), size));
            text.set_text(init_text.to_str()?.as_ref());
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
        });
        methods.add_method_mut("addTask", |_, this, (predicate, function): (Function, Function)| {
            let task = Task::new(predicate, function);
            let handle = task.handle();
            this.get().tasks.push(task);
            Ok(handle)
        });
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
            InputEvent::Char(c) => {
                table.set("type", "char")?;
                table.set("c", c as u32)?;
            }
        }
        Ok(Value::Table(table))
    }
}

pub struct UIManager {
    lua: Lua,
    window_size: (f32, f32),
    mouse_position: (f32, f32),
    context: UIRenderContextRef,
}

impl UIManager {
    pub fn new(nvg: &NvgInstance, window: &Window) -> Self {
        let lua = Lua::new();
        let globals = lua.globals();
        let context = UIRenderContextRef::new(nvg);
        globals.set("ui", context.clone()).unwrap();
        globals.set("dirty", true).unwrap();
        let (w, h) = window.get_framebuffer_size();
        Self {
            lua,
            context,
            window_size: (w as f32, h as f32),
            mouse_position: (0.0, 0.0),
        }
    }

    pub fn load_script(&mut self, chunk: impl AsChunk) -> Result<(), mlua::Error> {
        self.lua.load(chunk).exec()?;
        Ok(())
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn render(&self, width: f32, height: f32) -> Result<(), mlua::Error> {
        let globals = self.lua.globals();
        let ui = self.context.get();
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
            WindowEvent::Char(c) => {
                Some(InputEvent::Char(c))
            }
            _ => None
        };
        let ui = self.context.get();
        if let Some(event) = event {
            ui.handle_event(event)?;
        }
        Ok(())
    }

    pub fn set_dirty(&self) -> Result<(), mlua::Error> {
        let ui = self.context.get();
        ui.dirty = true;
        Ok(())
    }
}

impl Drop for UIManager {
    fn drop(&mut self) {
        self.context.deallocate()
    }
}