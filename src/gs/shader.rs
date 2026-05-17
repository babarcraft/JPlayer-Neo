use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::ptr::{null, null_mut};
use std::str::FromStr;
use gl::types::{GLchar, GLenum, GLint, GLsizei, GLuint};
use crate::gs::gl::check_errors;

pub enum UniformValue {
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Mat2([f32; 4]),
    Mat3([f32; 9]),
    Mat4([f32; 16]),
    Float(f32),
    Integer(i32),
}

pub struct Shader {
    id: GLuint,
}

impl Shader {
    fn compile_shader(source: &str, ty: GLenum) -> Result<GLuint, String> {
        unsafe {
            let shader = gl::CreateShader(ty);
            gl::ShaderSource(shader, 1, &CString::from_str(source).unwrap().as_ptr(), null());
            gl::CompileShader(shader);
            let mut result = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut result);
            if result != gl::TRUE as GLint {
                let mut buffer: [c_char; 1024] = [0; 1024];
                gl::GetShaderInfoLog(shader, buffer.len() as GLsizei, null_mut(), buffer.as_mut_ptr());
                return Err(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
            }
            Ok(shader)
        }
    }

    pub fn compile(vertex: &str, fragment: &str) -> Result<Shader, String> {
        unsafe {
            let vertex = Self::compile_shader(vertex, gl::VERTEX_SHADER)?;
            let fragment = Self::compile_shader(fragment, gl::FRAGMENT_SHADER)?;
            let id = gl::CreateProgram();
            gl::AttachShader(id, vertex);
            gl::AttachShader(id, fragment);
            gl::LinkProgram(id);
            let mut result = 0;
            gl::GetProgramiv(id, gl::LINK_STATUS, &mut result);
            if result != gl::TRUE as GLint {
                let mut buffer: [c_char; 1024] = [0; 1024];
                gl::GetProgramInfoLog(id, buffer.len() as GLsizei, null_mut(), buffer.as_mut_ptr());
                return Err(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
            }
            gl::DeleteShader(vertex);
            gl::DeleteShader(fragment);
            Ok(Shader { id })
        }
    }

    pub fn compile_compute(source: &str) -> Result<Shader, String> {
        unsafe {
            let shader = Self::compile_shader(source, gl::COMPUTE_SHADER)?;
            let id = gl::CreateProgram();
            gl::AttachShader(id, shader);
            gl::LinkProgram(id);
            let mut result = 0;
            gl::GetProgramiv(id, gl::LINK_STATUS, &mut result);
            if result != gl::TRUE as GLint {
                let mut buffer: [c_char; 1024] = [0; 1024];
                gl::GetProgramInfoLog(id, buffer.len() as GLsizei, null_mut(), buffer.as_mut_ptr());
                return Err(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
            }
            gl::DeleteShader(shader);
            Ok(Shader { id })
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::UseProgram(self.id);
        }
    }

    pub fn unbind(&self) {
        unsafe {
            gl::UseProgram(0);
        }
    }

    pub fn dispatch_compute(&self, size_x: u32, size_y: u32, size_z: u32) {
        unsafe {
            gl::DispatchCompute(size_x, size_y, size_z);
            check_errors("Compute dispatch", false);
        }
    }

    pub fn image_access_barrier(&self) {
        unsafe {
            gl::MemoryBarrier(gl::SHADER_IMAGE_ACCESS_BARRIER_BIT)
        }
    }

    pub fn get_uniform_location(&self, name: &str) -> Option<GLint> {
        unsafe {
            let location = gl::GetUniformLocation(self.id, CString::from_str(name).unwrap().as_ptr());
            if location < -1 {
                None
            } else {
                Some(location)
            }
        }
    }

    pub fn set_uniform(&self, location: GLint, value: &UniformValue) {
        unsafe {
            match value {
                UniformValue::Vec2(val) => {
                    gl::Uniform2fv(location, 1, val.as_ptr());
                }
                UniformValue::Vec3(val) => {
                    gl::Uniform3fv(location, 1, val.as_ptr());
                }
                UniformValue::Vec4(val) => {
                    gl::Uniform4fv(location, 1, val.as_ptr());
                }
                UniformValue::Mat2(val) => {
                    gl::UniformMatrix2fv(location, 1, gl::FALSE, val.as_ptr())
                }
                UniformValue::Mat3(val) => {
                    gl::UniformMatrix3fv(location, 1, gl::FALSE, val.as_ptr())
                }
                UniformValue::Mat4(val) => {
                    gl::UniformMatrix4fv(location, 1, gl::FALSE, val.as_ptr())
                }
                UniformValue::Float(val) => {
                    gl::Uniform1f(location, *val);
                }
                UniformValue::Integer(val) => {
                    gl::Uniform1i(location, *val);
                }
            }
        }
    }
}

impl Drop for Shader {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteProgram(self.id);
        }
    }
}