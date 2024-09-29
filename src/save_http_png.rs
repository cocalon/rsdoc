use clap::Parser;
use std::fs::{self, File};
use std::path::Path;
use std::time::Duration;

use deflate::deflate_bytes;

#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
struct Args {
    #[clap(help = "URL of the file to download")]
    url: String,
    #[clap(short, long, help = "Output filename")]
    output: Option<String>,
}

fn create_output_file(output_filename: &str) -> Result<File, String> {
    let output_file_path = Path::new(output_filename);

    // never overwrite the output file
    if output_file_path.exists() {
        return Err(format!(
            "output file {} already exists, aborting...",
            output_filename
        ));
    }

    match File::create(output_file_path) {
        Ok(file) => Ok(file),
        Err(e) => Err(format!(
            "failed to open file {} for writing; cause: {}",
            output_filename, e
        )),
    }
}

fn save_png(url:&str, output_filename:&str) -> bool{
    let _ = fs::remove_file(output_filename);

    let output_file_ret = create_output_file(&output_filename);
    if let Ok(mut output_file) = output_file_ret{
        let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(5))
        .timeout_write(Duration::from_secs(5))
        .build();

        let response = match agent.get(&url).call() {
            Ok(response) => response,
            Err(_) => {
                fs::remove_file(output_filename).unwrap();
                return false;
            }
        };

        match std::io::copy(&mut response.into_reader(), &mut output_file) {
            Ok(_) => (),
            Err(_) => {
                fs::remove_file(output_filename).unwrap();
                return false;
            }
        }   
    }
    return true;
}

pub fn encode64_(e:&[u8]) -> String{
    let mut r = String::new();

    for i in (0..e.len()).step_by(3)
    {
        if i+2==e.len(){
            r.push_str(append3bytes(e[i],e[i+1],0).as_str());
        }
        else
        {
            if i+1==e.len(){
                r.push_str(append3bytes(e[i],0,0).as_str());
            }
            else{
                r.push_str(append3bytes(e[i],e[i+1],e[i+2]).as_str());
            }
        }
    }
    r
}
    
fn append3bytes(e:u8,n:u8,t:u8) -> String
{
    let c1=e>>2;
    let c2=(3&e)<<4|n>>4;
    let c3 =(15&n)<<2|t>>6;
    let c4 =63&t;
    let mut r = String::new();
    r.push(encode6bit(63&c1));
    r.push(encode6bit(63&c2));
    r.push(encode6bit(63&c3));
    r.push(encode6bit(63&c4));  
    r
}
fn encode6bit(e1:u8) -> char{
    let mut e = e1;
    if e<10 {
        return char::from_u32( (e + 48) as u32).unwrap();
    }
    else{
        e -= 10;
        if e <26{
            return char::from_u32((65+e) as u32).unwrap();
        } 
        else{
            e -= 26;
            if e < 26{
                return char::from_u32((97+e) as u32).unwrap();
            }
            else{
                e -= 26;
                if 0== e{
                    return '-';
                }
                else{
                    if 1==e{
                        return '_';
                    }
                    else{
                        return '?';
                    }
                }
            }
        }
    }
}

pub fn download_puml(puml_str: &str, png_save_path_name:&str) -> bool
{
    //If the file_name encoded by sha1 still exists, it means that the UML code has not been modified, so return true directly
    if Path::new(&png_save_path_name).exists(){
        return true;
    }

    let path = std::path::Path::new(png_save_path_name);
    let prefix = path.parent().unwrap();
    std::fs::create_dir_all(prefix).unwrap();

    let compressed = deflate_bytes(puml_str.as_bytes());
    let encode64_str = encode64_(&compressed);
    let url = "http://www.plantuml.com/plantuml/png/".to_string() + &encode64_str;
  
    save_png(&url, png_save_path_name)
}