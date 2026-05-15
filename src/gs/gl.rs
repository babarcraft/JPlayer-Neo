use gl::types::GLenum;

#[derive(Debug)]
pub enum ErrorType {
    NoError,
    InvalidEnum,
    InvalidValue,
    InvalidOperation,
    InvalidFramebufferOperation,
    OutOfMemory,
    StackOverflow,
    StackUnderflow,
}

impl Into<ErrorType> for GLenum {
    fn into(self) -> ErrorType {
        match self {
            gl::NO_ERROR => ErrorType::NoError,
            gl::INVALID_ENUM => ErrorType::InvalidEnum,
            gl::INVALID_VALUE => ErrorType::InvalidValue,
            gl::INVALID_OPERATION => ErrorType::InvalidOperation,
            gl::INVALID_FRAMEBUFFER_OPERATION => ErrorType::InvalidFramebufferOperation,
            gl::OUT_OF_MEMORY => ErrorType::OutOfMemory,
            gl::STACK_OVERFLOW => ErrorType::StackOverflow,
            gl::STACK_UNDERFLOW => ErrorType::StackUnderflow,
            _ => ErrorType::NoError
        }
    }
}

pub fn check_errors(action: &'static str, panic: bool) {
    unsafe {
        loop {
            let error: ErrorType = gl::GetError().into();
            if let ErrorType::NoError = error {
                break
            } else if panic {
                panic!("OPENGL ERROR: {:?} during {action}", error)
            } else {
                println!("OPENGL ERROR: {:?} during {action}", error)
            }
        }
    }
}