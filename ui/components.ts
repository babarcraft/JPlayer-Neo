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
        this.getChildren().forEach(component => {
            switch(event.type) {
                case "mouseMoved": {
                    if(!component.intersects(event.to))
                        return
                    break
                }
                case "mouseButton": {
                    if(!component.intersects(event.pos))
                        return
                    break
                }
            }
            if(component.handleEvent) {
                let shouldFocus = event.type == "mouseButton";
                event = component.handleEvent(event)
                if(!event && shouldFocus) {
                    if(focused) focused.focused = false
                    component.focused = true
                    focused = component;
                }
            }
        })
        return event
    }
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
                if(component.render) component.render()
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
        this.children.forEach((value, index, array) => {
            let [x, y, w, h] = this.bounds();
            value.pos = { x: x, y: y };
            value.size = { w: w, h: h };
            if(value.id) {
                idMap[value.id] = value
            }
            if(value.render) {
                value.render()
            }
        })
    }
}

export class Label extends ComponentBase {
    text = ui.newText("", "default", 16.0)
    constructor() {
        super();
    }
    render() {
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
        this.text.pos = { x: x, y: y };
        this.text.size = { w: w, h: h };
        this.borderCommand = ui.pushIndirect({
            type: "shapeStrokeColor",
            shape: ["rect", x, y, w, h],
            color: this.cursorColor,
            width: 5
        });
        [x, y, w, h] = this.text.bounds(this.text.offset, this.text.offset);
        ui.push({
            type: "textFill",
            text: this.text,
            color: this.textColor
        })
        this.cursorCommand = ui.pushIndirect({
            type: "shapeFillColor",
            shape: ["rect", x, y, 5, h],
            color: this.cursorColor
        })
    }

    push(string: string) {
        this.text.push(string);
        this.update();
    }

    private update() {
        if(!this.cursorCommand) {
            console.log("Skip")
            return;
        }
        let [x, y, w, h] = this.text.bounds(this.text.offset, this.text.offset);
        if(this.cursorCommand.command.type == "shapeFillColor") {
            this.cursorCommand.command.shape = ["rect", x, y, 5, h]
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
            case "char":
                this.push(String.fromCharCode(event.c))
                break
            case "key":
                break
        }
        return null;
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

}

let root: ComponentBase | null = null;
let focused: ComponentBase | null = null;

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

ui.setEventHandle(event => {
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