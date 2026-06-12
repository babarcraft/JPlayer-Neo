type TextRange = [
    number?,
    number?
]

declare interface TextHandle {
    pos: Vec2;
    size?: Size;
    len: number;
    offset: number;
    text: string;
    fitType: "scale" | "range" | "rangeAndScale";

    bounds(begin?: number, end?: number): [number, number, number, number];
    push(str: string): void;
    pop(): void;
    clear(): void;
}

type Rect = [
    "rect",
    number,
    number,
    number,
    number
];
type Circle = [
    "circle",
    number,
    number,
    number
];

type Shape = Rect | Circle;

type Vec2 = {
    x?: number;
    y?: number;
};

type Size = {
    w?: number;
    h?: number;
};

type Font = {
    name: string,
    size: number
};

type Padding = {
    left?: number;
    right?: number;
    top?: number;
    bottom?: number;
};

type KeyEvent = {
    type: "key",
    key: number,
    action: number
}

type CharEvent = {
    type: "char",
    c: number,
}

type MouseMovedEvent = {
    type: "mouseMoved",
    from: Vec2,
    to: Vec2
}


type MouseButtonEvent = {
    type: "mouseButton",
    pos: Vec2,
    button: number,
    action: number,
}

type Event = KeyEvent | MouseButtonEvent | MouseMovedEvent | CharEvent;

type Color = [number, number, number, number];


declare interface VideoSurfaceHandle {}
declare interface InputWorker {}
declare interface DecodeWorker {}

declare interface VideoPlayerHandle {
    play(): void;
    seek(time: number): void;

    duration: number;
    pts: number;
    volume: number;
}

type CommandMap = {
    shapeFillColor: {
        shape: Shape;
        color: Color;
    };
    shapeStrokeColor: {
        shape: Shape;
        width: number;
        color: Color;
    };

    textFill: {
        text: TextHandle;
        color: Color;
    };

    videoSurface: {
        shape: Shape;
        surface: VideoSurfaceHandle;
    };

    indirect: {
        command: RenderCommand;
    };
};

type RenderCommand = {
    [K in keyof CommandMap]:
    { type: K } & CommandMap[K]
}[keyof CommandMap];

declare interface IndirectCommandHandle {
    command: RenderCommand;
    update(): void;
}

declare interface TaskHandle {
    canceled: boolean
}

declare interface UI {
    push(cmd: RenderCommand): void;
    pushIndirect(cmd: RenderCommand): IndirectCommandHandle;

    newText(
        text: string,
        font: string,
        size: number
    ): TextHandle;

    setText(
        text: TextHandle,
        value: string
    ): void;

    fitText(
        text: TextHandle,
        width?: number,
        height?: number
    ): void;

    newVideoSurface(): VideoSurfaceHandle;

    newVideoPlayer(
        path: string,
        surface: VideoSurfaceHandle,
        inputWorker: InputWorker,
        decodeWorker: DecodeWorker
    ): VideoPlayerHandle;

    newInputWorker(): InputWorker
    newDecodeWorker(): DecodeWorker

    getClipboard(): string
    
    getSize(): Size;

    setDirty(): void;

    setRenderHandle(handle: () => void): void;
    setEventHandle(handle: (this: void, event: Event) => void): void;
    setUpdateHandle(handle: () => void): void;

    addTask(predicate: (this: void, passed: number) => boolean, func: (this: void) => void): TaskHandle;
}

declare const ui: UI;

declare interface Console {
    log(...args: any[]): void;
    warn(...args: any[]): void;
    error(...args: any[]): void;
}

declare const console: Console;