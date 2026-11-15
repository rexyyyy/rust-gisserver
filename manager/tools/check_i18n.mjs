import fs from 'fs';
const j = JSON.parse(fs.readFileSync('D:/mycomputer/documents/gis-wt/thirdparty/rust-gisserver/manager/backend/locales/en_US.json', 'utf8'));
const keys = Object.keys(j.exact || {});
console.log('exact count:', keys.length);
console.log('has 登录成功:', Object.prototype.hasOwnProperty.call(j.exact, '登录成功'));
console.log('value:', JSON.stringify(j.exact['登录成功']));
// check "用户名或密码错误"
console.log('has 用户名或密码错误:', Object.prototype.hasOwnProperty.call(j.exact, '用户名或密码错误'));
console.log('all keys containing 登录:', keys.filter(k => k.includes('登录')));
