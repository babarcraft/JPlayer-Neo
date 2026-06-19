let idMap = new Map<string, ComponentBase>()

export class ComponentBase {
    pos?: Vec2;
    size?: Size;

    id?: string;

    prefPos?: Vec2;
    prefSize?: Size;

    padding?: Padding;
    parent?: ComponentBase;

    focused: boolean = false;
    _visible: boolean = true;

    set visible(vis: boolean) {
        if(this._visible != vis) {
            this._visible = vis
            ui.setDirty()
        }
    }

    get visible(): boolean {
        return this._visible
    }

    render?(): void;
    bounds(): [number, number, number, number] {
        let w = this.size?.w ?? 0;
        let h = this.size?.h ?? 0;

        let x = this.pos?.x ?? 0;
        let y = this.pos?.y ?? 0;

        if (this.prefSize) {
            w = this.prefSize.w ?? w;
            h = this.prefSize.h ?? h;
        }

        if (this.prefPos) {
            x = this.prefPos.x ?? x;
            y = this.prefPos.y ?? y;
        }

        const p = this.padding;
        if (p) {
            x += p.left ?? 0;
            y += p.bottom ?? 0;
            w -= (p.left ?? 0) + (p.right ?? 0);
            h -= (p.top ?? 0) + (p.bottom ?? 0);
        }

        return [x, y, w, h];
    }
    intersects(pos: Vec2) {
        let [x, y, w, h] = this.bounds();
        return pos.x >= x && pos.x <= x + w && pos.y >= y && pos.y <= y + h
    }

    also<T extends this>(fn: (obj: T) => void): T {
        fn(this as T);
        return this as T;
    }

    getChildren?(): ComponentBase[]

    handleEvent?(event: Event): Event | null {
        if(!this.getChildren) return event
        let children = this.getChildren()
        for(let i = Math.max(0, children.length - 1); i >= 0; i--) {
            let component = children[i];
            switch(event.type) {
                case "mouseMoved": {
                    if(!component.intersects(event.to))
                        continue
                    break
                }
                case "mouseButton": {
                    if(!component.intersects(event.pos))
                        continue
                    break
                }
            }
            if(component.handleEvent) {
                event = component.handleEvent(event)
                if(!event) { break }
            }
        }
        return event
    }
}

export function setFocused(component: ComponentBase): void {
    if(focused) focused.focused = false;
    focused = component;
    component.focused = true;
}

export type Direction = "horizontal" | "vertical"

export class Group extends ComponentBase {

    flow: Direction
    children: [number, ComponentBase | null][]

    constructor(flow: Direction, children: [number, ComponentBase | null][]) {
        super()
        this.flow = flow
        this.children = children
    }

    render() {
        if(!this.visible) return
        let sum = 0
        let [x, y, w, h] = this.bounds();
        this.children.forEach(value => {
            let [weight, ] = value
            sum += weight
        })
        this.children.forEach(value => {
            let [weight, component] = value
            weight /= sum
            let [dx, cw, dy, ch] = [0, 0, 0, 0]
            switch(this.flow) {
                case "horizontal":
                    dx = weight * w
                    cw = weight * w
                    ch = h
                    break
                case "vertical":
                    dy = weight * h
                    ch = weight * h
                    cw = w
                    break
            }

            if(component) {
                component.pos = { x: x, y: y }
                component.size = { w: cw, h: ch }
                if(component.render && component.visible) component.render()
            }

            x += dx
            y += dy
        })
    }

    getChildren(): ComponentBase[] {
        return this.children.filter(value => {
            let [, base] = value
            return base
        }).map(value => {
            let [, base] = value
            return base
        })
    }
}

export class Root extends ComponentBase {

    children: ComponentBase[]

    constructor(children: ComponentBase[]) {
        super();
        this.children = children;
    }

    getChildren(): ComponentBase[] {
        return this.children
    }

    render() {
        if(!this.visible) return
        this.children.forEach((value, index, array) => {
            let [x, y, w, h] = this.bounds();
            value.pos = { x: x, y: y };
            value.size = { w: w, h: h };
            if(value.id) {
                idMap[value.id] = value
            }
            if(value.render && value.visible) {
                value.render()
            }
        })
    }
}

export class Label extends ComponentBase {
    private readonly handle = ui.newText("", "default", 16.0)
    color: Color

    constructor(color: Color) {
        super();
        this.color = color;
        this.handle.fitType = "scale";
    }
    render() {
        let [x, y, w, h] = this.bounds();
        this.handle.pos = { x: x, y: y }
        this.handle.size = { w: w, h: h }
        this.alignText()
        ui.push({
            type: "textFill",
            text: this.handle,
            color: this.color
        })
    }

    private alignText() {
        let [tx, ty, tw, th] = this.handle.bounds()
        let [x, y, w, h] = this.bounds();
        this.handle.pos = { x: x + (w - tw) / 2, y: y + (h - th) / 2 }
    }

    set text(str: string) {
        this.handle.text = str
        this.alignText()
    }

    get text(): string {
        return this.handle.text
    }
}

export class TextInput extends ComponentBase {
    text = ui.newText("", "default", 16.0)
    cursorCommand: IndirectCommandHandle | null = null
    borderCommand: IndirectCommandHandle | null = null
    textColor: Color
    cursorColor: Color

    constructor(textColor: Color, cursorColor: Color) {
        super();
        this.textColor = textColor;
        this.cursorColor = cursorColor;
    }

    render() {
        let [x, y, w, h] = this.bounds();
        let [tx, ty, tw, th] = [x, y, w, h]
        if(tw > 10) { tx += 5; tw -= 10; }
        if(th > 10) { ty += 5; th -= 10; }
        this.text.pos = { x: tx, y: ty };
        this.text.size = { w: tw, h: th };
        this.borderCommand = ui.pushIndirect({
            type: "shapeStrokeColor",
            shape: ["rect", x, y, w, h],
            color: this.cursorColor,
            width: 2
        });
        [x, y, w, h] = this.text.bounds(this.text.offset, this.text.offset);
        ui.push({
            type: "textFill",
            text: this.text,
            color: this.textColor
        })
        this.cursorCommand = ui.pushIndirect({
            type: "shapeFillColor",
            shape: ["rect", tx, ty, 2, th],
            color: this.cursorColor
        })
        this.update();
    }

    push(string: string) {
        this.text.push(string);
        this.update();
    }

    private update() {
        if(!this.cursorCommand) {
            return;
        }
        let [x, y, w, h] = this.text.bounds(this.text.offset, this.text.offset);
        if(this.cursorCommand.command.type == "shapeFillColor") {
            this.cursorCommand.command.shape = ["rect", x, y, 2, h]
        }
        this.cursorCommand.update();
    }

    pop() {
        this.text.pop()
        this.update();
    }

    set offset(offset: number) {
        this.text.offset = offset
        this.update();
    }

    get offset() {
        return this.text.offset
    }

    handleEvent(event: Event): Event | null {
        switch(event.type) {
            case "mouseButton":
                setFocused(this)
                break
            case "char":
                this.push(String.fromCharCode(event.c))
                break
            case "key":
                this.handleKeyEvent(event)
                break
        }
        return null;
    }

    private controlPressed = false

    onEnterPressed?(): void

    handleKeyEvent(event: KeyEvent): void {
        switch(event.key) {
            case Key.Backspace:
                if(event.action != InputAction.Release)
                    this.pop()
                break
            case Key.Right:
                if(event.action != InputAction.Release)
                    this.offset += 1
                break
            case Key.Left:
                if(event.action != InputAction.Release)
                    this.offset -= 1
                break
            case Key.V:
                if(event.action != InputAction.Release && this.controlPressed) {
                    this.push(ui.getClipboard())
                }
                break
            case Key.LeftControl:
            case Key.RightControl:
                this.controlPressed = event.action != InputAction.Release;
                break
            case Key.Enter:
                if(event.action == InputAction.Press && this.onEnterPressed)
                    this.onEnterPressed()
                break
        }
    }
}

export class VideoSurface extends ComponentBase {

    surface: VideoSurfaceHandle

    constructor() {
        super();
        this.surface = ui.newVideoSurface();
    }

    render() {
        let [x, y, w, h] = this.bounds();
        ui.push({
            type: "videoSurface",
            shape: ["rect", x, y, w, h],
            surface: this.surface
        })
    }
}

export class Slider extends ComponentBase {

    private _progress: number = 0
    private indirect: IndirectCommandHandle | null = null
    backgroundColor: Color
    foregroundColor: Color
    private drag: boolean = false
    private readonly ws: [boolean, boolean]

    constructor(direction: Direction, backgroundColor: Color, foregroundColor: Color) {
        super();
        this.backgroundColor = backgroundColor;
        this.foregroundColor = foregroundColor;
        this.ws = [direction == "horizontal", direction == "vertical"]
    }

    render() {
        let [x, y, w, h] = this.bounds();
        let [wx, wy] = this.ws
        ui.push({
            type: "shapeFillColor",
            shape: ["rect", x, y, w, h],
            color: this.backgroundColor
        })
        this.indirect = ui.pushIndirect({
            type: "shapeFillColor",
            shape: ["rect", x, y, w * (wx? this._progress : 1), h * (wy? this._progress : 1)],
            color: this.foregroundColor
        })
    }

    set progress(p: number) {
        this._progress = p;
        if(!this.indirect) return
        let [wx, wy] = this.ws
        let command = this.indirect.command;
        let [x, y, w, h] = this.bounds();
        if(command.type == "shapeFillColor") {
            command.shape = ["rect", x, y, w * (wx? this._progress : 1), h * (wy? this._progress : 1)];
        }
        this.indirect.update();
    }

    get progress(): number {
        return this._progress
    }

    onNewTarget?(target: number): void;

    handleEvent(event: Event): Event | null {
        let [x, y, w, h] = this.bounds();
        switch(event.type) {
            case "mouseButton":
                this.drag = event.action == InputAction.Press || event.action == InputAction.Repeat
                break
            case "mouseMoved":
                if(this.onNewTarget && this.drag) {
                    this.onNewTarget((event.to.x - x) / w)
                }
        }
        return null;
    }

}

export enum InputAction {
    Release = 0,
    Press = 1,
    Repeat = 2,
}

export enum Key {
    Unknown = -1,

    Space = 32,
    Apostrophe = 39,
    Comma = 44,
    Minus = 45,
    Period = 46,
    Slash = 47,

    Digit0 = 48,
    Digit1 = 49,
    Digit2 = 50,
    Digit3 = 51,
    Digit4 = 52,
    Digit5 = 53,
    Digit6 = 54,
    Digit7 = 55,
    Digit8 = 56,
    Digit9 = 57,

    Semicolon = 59,
    Equal = 61,

    A = 65,
    B = 66,
    C = 67,
    D = 68,
    E = 69,
    F = 70,
    G = 71,
    H = 72,
    I = 73,
    J = 74,
    K = 75,
    L = 76,
    M = 77,
    N = 78,
    O = 79,
    P = 80,
    Q = 81,
    R = 82,
    S = 83,
    T = 84,
    U = 85,
    V = 86,
    W = 87,
    X = 88,
    Y = 89,
    Z = 90,

    LeftBracket = 91,
    Backslash = 92,
    RightBracket = 93,
    GraveAccent = 96,

    World1 = 161,
    World2 = 162,

    Escape = 256,
    Enter = 257,
    Tab = 258,
    Backspace = 259,
    Insert = 260,
    Delete = 261,

    Right = 262,
    Left = 263,
    Down = 264,
    Up = 265,

    PageUp = 266,
    PageDown = 267,
    Home = 268,
    End = 269,

    CapsLock = 280,
    ScrollLock = 281,
    NumLock = 282,
    PrintScreen = 283,
    Pause = 284,

    F1 = 290,
    F2 = 291,
    F3 = 292,
    F4 = 293,
    F5 = 294,
    F6 = 295,
    F7 = 296,
    F8 = 297,
    F9 = 298,
    F10 = 299,
    F11 = 300,
    F12 = 301,
    F13 = 302,
    F14 = 303,
    F15 = 304,
    F16 = 305,
    F17 = 306,
    F18 = 307,
    F19 = 308,
    F20 = 309,
    F21 = 310,
    F22 = 311,
    F23 = 312,
    F24 = 313,
    F25 = 314,

    Numpad0 = 320,
    Numpad1 = 321,
    Numpad2 = 322,
    Numpad3 = 323,
    Numpad4 = 324,
    Numpad5 = 325,
    Numpad6 = 326,
    Numpad7 = 327,
    Numpad8 = 328,
    Numpad9 = 329,

    NumpadDecimal = 330,
    NumpadDivide = 331,
    NumpadMultiply = 332,
    NumpadSubtract = 333,
    NumpadAdd = 334,
    NumpadEnter = 335,
    NumpadEqual = 336,

    LeftShift = 340,
    LeftControl = 341,
    LeftAlt = 342,
    LeftSuper = 343,

    RightShift = 344,
    RightControl = 345,
    RightAlt = 346,
    RightSuper = 347,

    Menu = 348,
}

export enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,

    Button4 = 3,
    Button5 = 4,
    Button6 = 5,
    Button7 = 6,
    Button8 = 7,
}

let root: ComponentBase | null = null;
export let focused: ComponentBase | null = null;

ui.setRenderHandle(() => {
    if(root) {
        root.pos = { x: 0, y: 0 }
        root.size = ui.getSize();
        idMap.clear();
        if(root.id)
            idMap[root.id] = root;
        root.render()
    }
})

export type EventListener = (this: void, event: Event) => Event | null;

export let eventListeners: EventListener[] = []

ui.setEventHandle(event => {
    for(let listener of eventListeners) {
        event = listener(event)
        if(!event) return
    }
    switch(event.type) {
        case "mouseButton":
            if(focused) focused.focused = false
            focused = null
        case "mouseMoved": {
            if(root && root.handleEvent) {
                root.handleEvent(event)
            }
        }
        case "char":
        case "key": {
            if(focused && focused.handleEvent) {
                focused.handleEvent(event)
            }
        }
    }
})

export function setRoot(comp: ComponentBase): void {
    root = comp
}